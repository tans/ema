use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::{path::Path, sync::Mutex};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub owner: String,
    pub agent: String,
    pub project: String,
    pub cwd: Option<String>,
    pub agent_session: Option<String>,
    pub subject: String,
    pub root_mid: String,
    pub last_mid: String,
}
#[derive(Debug, Clone)]
pub struct Job {
    pub id: String,
    pub session_id: String,
    pub prompt: String,
}
#[derive(Debug, Clone)]
pub struct Outbound {
    pub id: String,
    pub message_id: String,
    pub recipient: String,
    pub subject: String,
    pub in_reply_to: String,
    pub references: String,
    pub body: String,
    pub attempts: i64,
}
#[derive(Debug, Serialize)]
pub struct Stats {
    pub pending_jobs: i64,
    pub running_jobs: i64,
    pub received_messages: i64,
    pub last_activity: Option<i64>,
}

pub struct Database(pub Mutex<Connection>);
impl Database {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() { std::fs::create_dir_all(parent)?; }
        let c = Connection::open(path)?;
        c.busy_timeout(std::time::Duration::from_secs(3))?;
        c.execute_batch(r#"
            PRAGMA journal_mode=WAL;
            PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS sessions (
              id TEXT PRIMARY KEY, owner TEXT NOT NULL, agent TEXT NOT NULL,
              project TEXT NOT NULL, agent_session TEXT, subject TEXT NOT NULL,
              root_mid TEXT NOT NULL, last_mid TEXT NOT NULL, created_at INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS messages (
              message_id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id),
              direction TEXT NOT NULL, created_at INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS jobs (
              id TEXT PRIMARY KEY, session_id TEXT NOT NULL REFERENCES sessions(id),
              prompt TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'queued',
              started_at INTEGER, completed_at INTEGER, result TEXT);
            CREATE TABLE IF NOT EXISTS events (
              id INTEGER PRIMARY KEY AUTOINCREMENT, job_id TEXT NOT NULL REFERENCES jobs(id),
              created_at INTEGER NOT NULL, kind TEXT NOT NULL, summary TEXT);
            CREATE TABLE IF NOT EXISTS outbox (
              id TEXT PRIMARY KEY, message_id TEXT NOT NULL UNIQUE,
              recipient TEXT NOT NULL, subject TEXT NOT NULL,
              in_reply_to TEXT NOT NULL, refs TEXT NOT NULL,
              body TEXT NOT NULL, state TEXT NOT NULL DEFAULT 'queued',
              attempts INTEGER NOT NULL DEFAULT 0, next_attempt INTEGER NOT NULL DEFAULT 0);
            CREATE INDEX IF NOT EXISTS jobs_pending ON jobs(state);
            CREATE INDEX IF NOT EXISTS outbox_pending ON outbox(state, next_attempt);
        "#)?;
        let mut column_stmt=c.prepare("PRAGMA table_info(sessions)")?;
        let columns=column_stmt.query_map([], |r| r.get::<_,String>(1))?.collect::<rusqlite::Result<Vec<_>>>()?;
        drop(column_stmt);
        if !columns.iter().any(|name| name == "cwd") { c.execute("ALTER TABLE sessions ADD COLUMN cwd TEXT", [])?; }
        c.execute_batch("CREATE TABLE IF NOT EXISTS mail_cursors (mailbox TEXT PRIMARY KEY, last_uid INTEGER NOT NULL, uid_validity INTEGER NOT NULL);")?;
        // Never replay an interrupted agent run without human intervention.
        c.execute("UPDATE jobs SET state='interrupted', completed_at=strftime('%s','now') WHERE state='running'", [])?;
        Ok(Self(Mutex::new(c)))
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.0.lock().map_err(|_| anyhow::anyhow!("database mutex poisoned"))
    }
    fn map_session(r: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
        Ok(Session { id:r.get(0)?, owner:r.get(1)?, agent:r.get(2)?,
            project:r.get(3)?,agent_session:r.get(4)?,subject:r.get(5)?,
            root_mid:r.get(6)?,last_mid:r.get(7)?,cwd:r.get(8)? })
    }
    pub fn find_by_references(&self, refs: &[String]) -> Result<Option<Session>> {
        let c = self.lock()?;
        for mid in refs {
            let result = c.query_row(
                "SELECT s.id,s.owner,s.agent,s.project,s.agent_session,s.subject,s.root_mid,s.last_mid,s.cwd
                FROM messages m JOIN sessions s ON m.session_id=s.id WHERE m.message_id=?1",
                [mid], Self::map_session).optional()?;
            if result.is_some() { return Ok(result); }
        }
        Ok(None)
    }
    pub fn session(&self, id: &str) -> Result<Session> {
        self.lock()?.query_row(
            "SELECT id,owner,agent,project,agent_session,subject,root_mid,last_mid,cwd FROM sessions WHERE id=?1",
            [id], Self::map_session).with_context(|| format!("session not found: {id}"))
    }
    pub fn store_inbound(&self, mid: &str, owner: &str, existing: Option<&Session>,
        agent: &str, project: &str, subject: &str, prompt: &str, cwd: Option<&str>, enqueue: bool) -> Result<Option<String>> {
        let mut c = self.lock()?;
        let tx = c.transaction()?;
        let session_id = existing.map(|s| s.id.clone()).unwrap_or_else(|| Uuid::new_v4().to_string());
        if existing.is_none() {
            tx.execute("INSERT INTO sessions(id,owner,agent,project,subject,root_mid,last_mid,created_at,cwd)
                VALUES(?1,?2,?3,?4,?5,?6,?6,strftime('%s','now'),?7)",
                params![session_id, owner,agent,project,subject,mid,cwd])?;
        }
        let added = tx.execute("INSERT OR IGNORE INTO messages(message_id,session_id,direction,created_at)
            VALUES (?1,?2,'in',strftime('%s','now'))", params![mid,session_id])?;
        if added == 0 { tx.rollback()?; return Ok(None); }
        tx.execute("UPDATE sessions SET last_mid=?1 WHERE id=?2",params![mid,session_id])?;
        tx.execute("INSERT INTO jobs(id,session_id,prompt,state) VALUES(?1,?2,?3,?4)",
            params![Uuid::new_v4().to_string(),session_id,prompt,if enqueue {"queued"} else {"awaiting_directory"}])?;
        tx.commit()?;
        Ok(Some(session_id))
    }
    pub fn pending_prompt(&self, session_id:&str) -> Result<Option<String>> {
        let c=self.lock()?;
        Ok(c.query_row("SELECT prompt FROM jobs WHERE session_id=?1 AND state='awaiting_directory' ORDER BY rowid LIMIT 1",
            [session_id],|r|r.get(0)).optional()?)
    }
    pub fn resolve_pending(&self, session_id:&str)->Result<()> {
        self.lock()?.execute("UPDATE jobs SET state='resolved' WHERE session_id=?1 AND state='awaiting_directory'",[session_id])?;
        Ok(())
    }
    pub fn update_cwd(&self, session_id: &str, cwd: &str) -> Result<()> {
        self.lock()?.execute("UPDATE sessions SET cwd=?1 WHERE id=?2 AND cwd IS NULL", params![cwd,session_id])?;
        Ok(())
    }
    pub fn known_workspaces(&self) -> Result<Vec<String>> {
        let c=self.lock()?;
        let mut st=c.prepare("SELECT DISTINCT cwd FROM sessions WHERE cwd IS NOT NULL")?;
        let rows=st.query_map([],|r|r.get::<_,String>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn cursor(&self, mailbox: &str) -> Result<(u32,u32)> {
        Ok(self.lock()?.query_row("SELECT last_uid,uid_validity FROM mail_cursors WHERE mailbox=?1", [mailbox], |r|Ok((r.get(0)?,r.get(1)?))).optional()?.unwrap_or((0,0)))
    }
    pub fn set_cursor(&self, mailbox:&str, uid:u32, validity:u32) -> Result<()> {
        self.lock()?.execute("INSERT INTO mail_cursors(mailbox,last_uid,uid_validity) VALUES(?1,?2,?3) ON CONFLICT(mailbox) DO UPDATE SET last_uid=excluded.last_uid,uid_validity=excluded.uid_validity",params![mailbox,uid,validity])?;
        Ok(())
    }
    pub fn claim(&self) -> Result<Option<Job>> {
        let mut c = self.lock()?;
        let tx = c.transaction()?;
        let found = tx.query_row("SELECT id,session_id,prompt FROM jobs WHERE state='queued'
            ORDER BY rowid ASC LIMIT 1", [], |r| Ok(Job {
                id:r.get(0)?,session_id:r.get(1)?,prompt:r.get(2)?
            })).optional()?;
        if let Some(ref job)=found {
            tx.execute("UPDATE jobs SET state='running',started_at=strftime('%s','now') WHERE id=?1", [&job.id])?;
        }
        tx.commit()?;
        Ok(found)
    }
    pub fn set_agent_session(&self, session_id: &str, agent_session: &str) -> Result<()> {
        self.lock()?.execute("UPDATE sessions SET agent_session=?1 WHERE id=?2",params![agent_session,session_id])?;
        Ok(())
    }
    pub fn event(&self, job_id: &str, kind: &str, summary: &str) -> Result<()> {
        self.lock()?.execute("INSERT INTO events(job_id,created_at,kind,summary)
            VALUES(?1,strftime('%s','now'),?2,?3)",params![job_id,kind,summary])?;
        Ok(())
    }
    pub fn finish(&self, job_id: &str, ok: bool, result: &str) -> Result<()> {
        self.lock()?.execute("UPDATE jobs SET state=?1,completed_at=strftime('%s','now'),result=?2
            WHERE id=?3", params![if ok {"completed"} else {"failed"},result,job_id])?;
        Ok(())
    }
    pub fn queue_reply(&self, session_id: &str, mailbox: &str, body: &str) -> Result<()> {
        let mut c = self.lock()?;
        let tx = c.transaction()?;
        let session = tx.query_row(
            "SELECT id,owner,agent,project,agent_session,subject,root_mid,last_mid,cwd FROM sessions WHERE id=?1",
            [session_id], Self::map_session)?;
        let domain = mailbox.split('@').nth(1).unwrap_or("localhost");
        let mid = format!("<ema-{}@{}>",Uuid::new_v4(), domain);
        let refs = if session.root_mid == session.last_mid { session.root_mid.clone() }
            else { format!("{} {}",session.root_mid,session.last_mid) };
        tx.execute("INSERT INTO outbox(id,message_id,recipient,subject,in_reply_to,refs,body)
            VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![Uuid::new_v4().to_string(),mid,session.owner,format!("Re: {}",session.subject),
                session.last_mid, refs,body])?;
        tx.execute("INSERT INTO messages(message_id,session_id,direction,created_at)
            VALUES(?1,?2,'out',strftime('%s','now'))",params![mid,session_id])?;
        tx.execute("UPDATE sessions SET last_mid=?1 WHERE id=?2",params![mid,session_id])?;
        tx.commit()?;
        Ok(())
    }
    pub fn pending_outbox(&self) -> Result<Vec<Outbound>> {
        let c=self.lock()?;
        let mut st=c.prepare("SELECT id,message_id,recipient,subject,in_reply_to,refs,body,attempts
            FROM outbox WHERE state='queued' AND next_attempt <= strftime('%s','now')
            ORDER BY rowid LIMIT 20")?;
        let rows=st.query_map([],|r| Ok(Outbound {id:r.get(0)?,message_id:r.get(1)?,
            recipient:r.get(2)?,subject:r.get(3)?,in_reply_to:r.get(4)?,
            references:r.get(5)?,body:r.get(6)?,attempts:r.get(7)?}))?;
        let outgoing=rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(outgoing)
    }
    pub fn outbox_result(&self,id:&str,success:bool,attempts:i64)->Result<()> {
        if success {
            self.lock()?.execute("UPDATE outbox SET state='sent' WHERE id=?1",[id])?;
        } else {
            let delay=60_i64.saturating_mul(1_i64 << attempts.min(6));
            self.lock()?.execute("UPDATE outbox SET attempts=attempts+1,
                next_attempt=strftime('%s','now')+?2 WHERE id=?1",params![id,delay])?;
        }
        Ok(())
    }
    pub fn stats(&self)->Result<Stats> {
        let c=self.lock()?;
        Ok(Stats{
            pending_jobs:c.query_row("SELECT COUNT(*) FROM jobs WHERE state='queued'",[],|r|r.get(0))?,
            running_jobs:c.query_row("SELECT COUNT(*) FROM jobs WHERE state='running'",[],|r|r.get(0))?,
            received_messages:c.query_row("SELECT COUNT(*) FROM messages WHERE direction='in'",[],|r|r.get(0))?,
            last_activity:c.query_row("SELECT MAX(created_at) FROM messages",[],|r|r.get(0))?,
        })
    }
}
