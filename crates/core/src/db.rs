use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use std::{path::Path, sync::Mutex};
use uuid::Uuid;
use crate::email_format::{self as fmt, text};

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
    pub kind: String,
}
#[derive(Debug)]
pub struct HookReport {
    pub message_id: String,
    pub subject: String,
    pub body: String,
    pub agent: String,
    pub native_session: String,
    pub cwd: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct MailRecord {
    pub id: String,
    pub message_id: String,
    pub direction: String,
    pub sender: String,
    pub mailbox: String,
    pub recipient: String,
    pub cc: String,
    pub references: String,
    pub original_date: String,
    pub subject: String,
    pub body: String,
    pub status: String,
    pub detail: String,
    pub archive_note: String,
    pub created_at: Option<i64>,
    pub sent_at: Option<i64>,
    pub attempts: Option<i64>,
    pub kind: String,
}
#[derive(Debug, Serialize)]
pub struct Stats {
    pub hook_reports: i64,
    pub hook_sent: i64,
    pub hook_pending: i64,
    pub total_sessions: i64,
    pub completed_jobs: i64,
    pub failed_jobs: i64,
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
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
            CREATE TABLE IF NOT EXISTS mail_inbox (
              mailbox TEXT NOT NULL, uid_validity INTEGER NOT NULL, uid INTEGER NOT NULL,
              message_id TEXT NOT NULL, sender TEXT NOT NULL, recipient TEXT NOT NULL,
              cc TEXT NOT NULL, refs TEXT NOT NULL, original_date TEXT NOT NULL, subject TEXT NOT NULL,
              body TEXT NOT NULL, status TEXT NOT NULL, detail TEXT NOT NULL DEFAULT '',
              archive_note TEXT NOT NULL DEFAULT '',
              created_at INTEGER NOT NULL,
              PRIMARY KEY(mailbox, uid_validity, uid));
            CREATE TABLE IF NOT EXISTS outbox_attempts (
              id INTEGER PRIMARY KEY AUTOINCREMENT, outbox_id TEXT NOT NULL REFERENCES outbox(id),
              attempted_at INTEGER NOT NULL, state TEXT NOT NULL, error TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS jobs_pending ON jobs(state);
            CREATE INDEX IF NOT EXISTS outbox_pending ON outbox(state, next_attempt);
        "#)?;
        #[cfg(unix)]
        for suffix in ["-wal", "-shm"] {
            use std::os::unix::fs::PermissionsExt;
            let mut name=path.as_os_str().to_os_string();
            name.push(suffix);
            if let Err(error)=std::fs::set_permissions(name, std::fs::Permissions::from_mode(0o600)) {
                if error.kind() != std::io::ErrorKind::NotFound { return Err(error.into()); }
            }
        }
        let mut column_stmt=c.prepare("PRAGMA table_info(sessions)")?;
        let columns=column_stmt.query_map([], |r| r.get::<_,String>(1))?.collect::<rusqlite::Result<Vec<_>>>()?;
        drop(column_stmt);
        if !columns.iter().any(|name| name == "cwd") { c.execute("ALTER TABLE sessions ADD COLUMN cwd TEXT", [])?; }
        c.execute_batch("CREATE TABLE IF NOT EXISTS mail_cursors (mailbox TEXT PRIMARY KEY, last_uid INTEGER NOT NULL, uid_validity INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS local_replies (
                message_id TEXT PRIMARY KEY, sender TEXT NOT NULL, mailbox TEXT NOT NULL,
                refs TEXT NOT NULL, subject TEXT NOT NULL, body TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'processing', detail TEXT NOT NULL DEFAULT '',
                kind TEXT NOT NULL, created_at INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS hook_events (fingerprint TEXT PRIMARY KEY, created_at INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS routing_replies (incoming_mid TEXT PRIMARY KEY, created_at INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS hook_context (message_id TEXT PRIMARY KEY, agent TEXT NOT NULL, native_session TEXT NOT NULL, cwd TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS message_aliases (
              alias_mid TEXT PRIMARY KEY, ema_mid TEXT NOT NULL, session_id TEXT,
              created_at INTEGER NOT NULL);
        ")?;
        let mut outbox_stmt=c.prepare("PRAGMA table_info(outbox)")?;
        let outbox_cols=outbox_stmt.query_map([], |r|r.get::<_,String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(outbox_stmt);
        if !outbox_cols.iter().any(|name|name=="source") {
            c.execute("ALTER TABLE outbox ADD COLUMN source TEXT NOT NULL DEFAULT 'task'",[])?;
        }
        for (name, definition) in [
            ("sender", "TEXT NOT NULL DEFAULT ''"),
            ("created_at", "INTEGER"),
            ("sent_at", "INTEGER"),
            ("last_error", "TEXT NOT NULL DEFAULT ''"),
            ("kind", "TEXT NOT NULL DEFAULT ''"),
        ] {
            if !outbox_cols.iter().any(|column| column == name) {
                c.execute(&format!("ALTER TABLE outbox ADD COLUMN {name} {definition}"), [])?;
            }
        }
        // Classify legacy outbox rows by source so history badges keep working after the kind migration.
        c.execute("UPDATE outbox SET kind='hook' WHERE kind='' AND source='hook'", [])?;
        c.execute("UPDATE outbox SET kind='test' WHERE kind='' AND source='test'", [])?;
        c.execute("UPDATE outbox SET kind='result' WHERE kind='' AND source='task'", [])?;
        let mut inbox_stmt=c.prepare("PRAGMA table_info(mail_inbox)")?;
        let inbox_cols=inbox_stmt.query_map([], |r|r.get::<_,String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(inbox_stmt);
        if !inbox_cols.iter().any(|name|name=="kind") {
            c.execute("ALTER TABLE mail_inbox ADD COLUMN kind TEXT NOT NULL DEFAULT 'unrelated'",[])?;
        }
        // Older threaded replies have a creation time in messages; other old rows do not.
        c.execute("UPDATE outbox SET created_at=(SELECT created_at FROM messages WHERE messages.message_id=outbox.message_id) WHERE created_at IS NULL", [])?;
        // A test interrupted after SMTP acceptance cannot safely be sent again automatically.
        c.execute("UPDATE outbox SET state='unknown' WHERE source='test' AND state='sending'", [])?;
        let mut jobs_stmt=c.prepare("PRAGMA table_info(jobs)")?;
        let jobs_cols=jobs_stmt.query_map([], |r|r.get::<_,String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(jobs_stmt);
        if !jobs_cols.iter().any(|name|name=="ack_outbox_id") {
            c.execute("ALTER TABLE jobs ADD COLUMN ack_outbox_id TEXT",[])?;
        }
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
            let resolved = self.resolve_message_id(&c, mid)?;
            let result = c.query_row(
                "SELECT s.id,s.owner,s.agent,s.project,s.agent_session,s.subject,s.root_mid,s.last_mid,s.cwd
                FROM messages m JOIN sessions s ON m.session_id=s.id WHERE m.message_id=?1",
                [resolved], Self::map_session).optional()?;
            if result.is_some() { return Ok(result); }
        }
        Ok(None)
    }
    /// QQ Mail rewrites Message-IDs of outbound mail (`<tencent_...@qq.com>`), so a real
    /// reply references the rewritten ID, not the `<ema-...>` ID we stored. Resolve through
    /// the alias table recorded when our own outbound mail came back as an echo.
    fn resolve_message_id(&self, c: &rusqlite::Connection, mid: &str) -> Result<String> {
        let ema: Option<String> = c.query_row(
            "SELECT ema_mid FROM message_aliases WHERE alias_mid=?1", [mid],
            |r| r.get(0)).optional()?;
        Ok(ema.unwrap_or_else(|| mid.to_string()))
    }
    /// Record that `<alias_mid>` (e.g. a tencent-rewritten Message-ID) actually refers to
    /// `<ema_mid>` (the original id we generated). session_id fills in where known.
    pub fn record_alias(&self, alias_mid: &str, ema_mid: &str) -> Result<()> {
        if alias_mid == ema_mid || alias_mid.trim().is_empty() { return Ok(()); }
        let session_id: Option<String> = self.lock()?.query_row(
            "SELECT session_id FROM messages WHERE message_id=?1", [ema_mid],
            |r| r.get(0)).optional()?;
        self.lock()?.execute(
            "INSERT OR IGNORE INTO message_aliases(alias_mid,ema_mid,session_id,created_at)
             VALUES(?1,?2,?3,strftime('%s','now'))",
            params![alias_mid, ema_mid, session_id])?;
        Ok(())
    }
    pub fn set_inbound_kind(&self, message_id: &str, kind: &str) -> Result<()> {
        self.lock()?.execute("UPDATE mail_inbox SET kind=?2 WHERE message_id=?1",
            params![message_id, kind])?;
        Ok(())
    }
    pub fn is_outbound(&self, mid: &str) -> Result<bool> {
        Ok(self.lock()?.query_row("SELECT EXISTS(SELECT 1 FROM outbox WHERE message_id=?1)",
            [mid], |r|r.get::<_,i64>(0))? != 0)
    }
    pub fn find_hook_report(&self, refs: &[String]) -> Result<Option<HookReport>> {
        let c = self.lock()?;
        for mid in refs {
            let resolved = self.resolve_message_id(&c, mid)?;
            let row: Option<(String,String,Option<String>,Option<String>,Option<String>)> = c.query_row(
                "SELECT o.subject,o.body,h.agent,h.native_session,h.cwd FROM outbox o
                 LEFT JOIN hook_context h ON h.message_id=o.message_id
                 WHERE o.message_id=?1 AND o.source='hook'",
                [resolved], |r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
            if let Some((subject,body,agent,native_session,cwd))=row {
                let field=|key:&str| body.lines().find_map(|line|line.strip_prefix(key))
                    .unwrap_or("").trim().to_string();
                return Ok(Some(HookReport { message_id:mid.clone(), subject,
                    agent:agent.unwrap_or_else(||field("Agent: ")),
                    native_session:native_session.unwrap_or_else(||field("Session: ")),
                    cwd:cwd.filter(|p|!p.is_empty()), body }));
            }
        }
        Ok(None)
    }
    pub fn session(&self, id: &str) -> Result<Session> {
        self.lock()?.query_row(
            "SELECT id,owner,agent,project,agent_session,subject,root_mid,last_mid,cwd FROM sessions WHERE id=?1",
            [id], Self::map_session).with_context(|| format!("session not found: {id}"))
    }
    pub fn store_inbound(&self, mid: &str, owner: &str, existing: Option<&Session>,
        agent: &str, project: &str, subject: &str, prompt: &str, cwd: Option<&str>, enqueue: bool,
        mailbox: &str, task: &str, hook: Option<&HookReport>, language: &str) -> Result<Option<String>> {
        let mut c = self.lock()?;
        let tx = c.transaction()?;
        let session_id = existing.map(|s| s.id.clone()).unwrap_or_else(|| Uuid::new_v4().to_string());
        if existing.is_none() {
            let root=hook.map(|report|report.message_id.as_str()).unwrap_or(mid);
            let original_subject=hook.map(|report|report.subject.as_str()).unwrap_or(subject);
            tx.execute("INSERT INTO sessions(id,owner,agent,project,subject,root_mid,last_mid,created_at,cwd,agent_session)
                VALUES(?1,?2,?3,?4,?5,?6,?6,strftime('%s','now'),?7,?8)",
                params![session_id, owner,agent,project,original_subject,root,cwd,hook.map(|report|report.native_session.as_str())])?;
            if hook.is_some() {
                tx.execute("INSERT INTO messages(message_id,session_id,direction,created_at)
                    VALUES (?1,?2,'out',strftime('%s','now'))",params![root,session_id])?;
            }
        }
        let added = tx.execute("INSERT OR IGNORE INTO messages(message_id,session_id,direction,created_at)
            VALUES (?1,?2,'in',strftime('%s','now'))", params![mid,session_id])?;
        if added == 0 { tx.rollback()?; return Ok(None); }
        // Mail classification: a message that reaches this point is a task — either the
        // start of a new session or a continuation of an existing one.
        tx.execute("UPDATE mail_inbox SET kind=?2 WHERE message_id=?1",
            params![mid, if existing.is_some() || hook.is_some() {"resume_task"} else {"new_task"}])?;
        tx.execute("UPDATE sessions SET last_mid=?1 WHERE id=?2",params![mid,session_id])?;
        let job_id=Uuid::new_v4().to_string();
        let outbox_id=Uuid::new_v4().to_string();
        let status = if enqueue { text(language, "已排队", "Queued") } else { text(language, "待补充信息，尚未执行", "Awaiting input; not started") };
        let native = existing.and_then(|s| s.agent_session.clone()).or_else(|| hook.map(|r| r.native_session.clone()));
        let session = Session { id: session_id.clone(), owner: owner.into(), agent: agent.into(), project: project.into(),
            cwd: cwd.map(str::to_owned), agent_session: native, subject: subject.into(), root_mid: String::new(), last_mid: String::new() };
        let next = if enqueue { text(language, "回执发送后开始处理，请等待结果。可直接回复补充要求。", "Processing starts after this acknowledgment is sent. Wait for the result or reply with additional instructions.") }
            else { text(language, "请直接回复项目名称或项目目录。", "Reply with the project name or working directory.") };
        let permission = if agent == "alma" { text(language, "只读；本轮不调用工具", "Read-only; no tools in this turn") } else { "" };
        let body = fmt::task_report(language, &session, &job_id, text(language, "已收到本次指令。", "Your instruction has been received."), status,
            &[(text(language, "本次指令", "Instruction"), &fmt::excerpt(task, fmt::TASK_LIMIT, language)),
              (text(language, "权限", "Permissions"), permission)], next);
        let domain=mailbox.split('@').nth(1).unwrap_or("localhost");
        let out_mid=format!("<ema-{}@{}>",Uuid::new_v4(),domain);
        let root=existing.map(|s|s.root_mid.as_str())
            .or_else(||hook.map(|report|report.message_id.as_str())).unwrap_or(mid);
        let refs=if root==mid {root.to_string()} else {format!("{root} {mid}")};
        let reply_subject=existing.map(|s|s.subject.as_str())
            .or_else(||hook.map(|report|report.subject.as_str())).unwrap_or(subject);
        let ack_kind=if existing.is_some() || hook.is_some() {"resume_ack"} else {"new_task_ack"};
        tx.execute("INSERT INTO outbox(id,message_id,recipient,subject,in_reply_to,refs,body,sender,created_at,kind)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,strftime('%s','now'),?9)",
            params![outbox_id,out_mid,owner,fmt::reply_subject(reply_subject),mid,refs,fmt::envelope(ack_kind,&out_mid,&body),mailbox,ack_kind])?;
        tx.execute("INSERT INTO messages(message_id,session_id,direction,created_at)
            VALUES(?1,?2,'out',strftime('%s','now'))",params![out_mid,session_id])?;
        tx.execute("UPDATE sessions SET last_mid=?1 WHERE id=?2",params![out_mid,session_id])?;
        tx.execute("INSERT INTO jobs(id,session_id,prompt,state,ack_outbox_id) VALUES(?1,?2,?3,?4,?5)",
            params![job_id,session_id,prompt,if enqueue {"queued"} else {"awaiting_directory"},outbox_id])?;
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
    pub fn record_incoming(&self, mailbox: &str, validity: u32, mail: &crate::mail::FetchedMail) -> Result<()> {
        self.lock()?.execute("INSERT OR IGNORE INTO mail_inbox
            (mailbox,uid_validity,uid,message_id,sender,recipient,cc,refs,original_date,subject,body,status,archive_note,kind,created_at)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,'processing',?12,'unrelated',strftime('%s','now'))",
            params![mailbox,validity,mail.uid,mail.message_id,mail.sender,mail.recipient,mail.cc,
                mail.references,mail.original_date,mail.subject,mail.body,mail.archive_note])?;
        Ok(())
    }
    pub fn incoming_result(&self, mailbox: &str, validity: u32, uid: u32, status: &str, detail: &str) -> Result<()> {
        let (kind, reason) = match status {
            "echo" => (Some("echo"), "EMA-generated message; no task or reply"),
            "ordinary" => (Some("unrelated"), "No recognized Agent prefix or known conversation reference"),
            "automated" => (Some("unrelated"), "Automated or bulk message"),
            "unauthorized" => (Some("unrelated"), "Sender is not allowlisted"),
            "empty" | "access_denied" | "owner_mismatch" | "unsupported"
                | "not_installed" | "unavailable_runner" | "missing_session" | "malformed"
                | "too_large_for_task" | "unavailable" => (Some("invalid"), status),
            _ => (None, status),
        };
        let detail = if detail.is_empty() { reason } else { detail };
        self.lock()?.execute("UPDATE mail_inbox SET status=?4,detail=?5,kind=COALESCE(?6,kind) WHERE mailbox=?1 AND uid_validity=?2 AND uid=?3 AND status IN ('processing','retry')",
            params![mailbox,validity,uid,status,detail,kind])?;
        Ok(())
    }
    pub fn mail_history(&self, search: &str, offset: i64) -> Result<Vec<MailRecord>> {
        self.mail_records(search, offset, "")
    }
    pub fn mail_record(&self, id: &str) -> Result<Option<MailRecord>> {
        Ok(self.mail_records("", 0, id)?.into_iter().next())
    }
    fn mail_records(&self, search: &str, offset: i64, id: &str) -> Result<Vec<MailRecord>> {
        let c = self.lock()?;
        let mut stmt = c.prepare("SELECT id,message_id,direction,sender,mailbox,recipient,cc,refs,original_date,subject,body,status,detail,archive_note,created_at,sent_at,attempts,kind FROM (
            SELECT 'in:'||mailbox||':'||uid_validity||':'||uid AS id,message_id,'in' AS direction,
              sender,mailbox,recipient,cc,refs,original_date,subject,body,status,detail,archive_note,created_at,NULL AS sent_at,0 AS attempts,kind
              FROM mail_inbox
            UNION ALL
            SELECT 'out:'||id,message_id,'out',sender,'',recipient,'',refs,'',subject,body,state,last_error,'',created_at,sent_at,
              CASE WHEN state='sent' AND sent_at IS NULL THEN NULL ELSE attempts END,kind FROM outbox
            UNION ALL
            SELECT 'local:'||message_id,message_id,'in',sender,mailbox,mailbox,'',refs,'',subject,body,status,detail,'local_reply',created_at,NULL,0,kind FROM local_replies
            UNION ALL
            SELECT 'legacy:'||m.message_id,m.message_id,'in',s.owner,'','','','','','','','legacy','','legacy',m.created_at,NULL,0,'legacy'
              FROM messages m JOIN sessions s ON m.session_id=s.id
              WHERE m.direction='in' AND NOT EXISTS (SELECT 1 FROM mail_inbox i WHERE i.message_id=m.message_id)
                AND NOT EXISTS (SELECT 1 FROM local_replies l WHERE l.message_id=m.message_id)
            ) WHERE (?3='' OR id=?3) AND (?1='' OR instr(lower(message_id||' '||sender||' '||mailbox||' '||recipient||' '||cc||' '||refs||' '||subject||' '||body),lower(?1))>0)
            ORDER BY created_at IS NULL,created_at DESC,id DESC LIMIT 51 OFFSET ?2")?;
        let rows=stmt.query_map(params![search.trim(),offset.max(0),id],|r|Ok(MailRecord {
            id:r.get(0)?,message_id:r.get(1)?,direction:r.get(2)?,sender:r.get(3)?,mailbox:r.get(4)?,
            recipient:r.get(5)?,cc:r.get(6)?,references:r.get(7)?,original_date:r.get(8)?,
            subject:r.get(9)?,body:r.get(10)?,status:r.get(11)?,detail:r.get(12)?,archive_note:r.get(13)?,
            created_at:r.get(14)?,sent_at:r.get(15)?,attempts:r.get(16)?,kind:r.get(17)?,
        }))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn record_local_reply(&self, mid:&str, sender:&str, mailbox:&str, reference:&str, subject:&str, body:&str) -> Result<()> {
        let c=self.lock()?;
        c.execute("INSERT OR IGNORE INTO local_replies(message_id,sender,mailbox,refs,subject,body,kind,created_at)
            VALUES(?1,?2,?3,?4,?5,?6,'resume_task',strftime('%s','now'))",params![mid,sender,mailbox,reference,subject,body])?;
        let same:bool=c.query_row("SELECT sender=?2 AND mailbox=?3 AND refs=?4 AND body=?5 FROM local_replies WHERE message_id=?1",
            params![mid,sender,mailbox,reference,body],|r|r.get(0))?;
        anyhow::ensure!(same,"Reply request ID was already used for another instruction");
        Ok(())
    }
    pub fn local_reply_result(&self, mid:&str, status:&str, detail:&str) -> Result<()> {
        self.lock()?.execute("UPDATE local_replies SET status=?2,detail=?3,kind=CASE WHEN ?2 IN ('queued','duplicate','awaiting_directory') THEN 'resume_task' ELSE 'invalid' END WHERE message_id=?1",params![mid,status,detail])?;
        Ok(())
    }
    pub fn claim(&self) -> Result<Option<Job>> {
        let mut c = self.lock()?;
        let tx = c.transaction()?;
        let found = tx.query_row("SELECT id,session_id,prompt FROM jobs WHERE state='queued'
            AND (ack_outbox_id IS NULL OR EXISTS(SELECT 1 FROM outbox WHERE outbox.id=jobs.ack_outbox_id AND outbox.state='sent'))
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
    /// Record one failure receipt per incoming Message-ID, even across IMAP retries.
    /// No Agent session is created for unknown/missing execution adapters.
    pub fn queue_route_error(&self, mid:&str, recipient:&str, mailbox:&str,
        subject:&str, error:&str, references:&[String]) -> Result<bool> {
        let mut c=self.lock()?;
        let tx=c.transaction()?;
        let added=tx.execute(
            "INSERT OR IGNORE INTO routing_replies(incoming_mid,created_at) VALUES(?1,strftime('%s','now'))",
            [mid])?;
        if added == 0 {tx.rollback()?;return Ok(false);}
        let domain=mailbox.split('@').nth(1).unwrap_or("localhost");
        let reply_mid=format!("<ema-error-{}@{}>",Uuid::new_v4(),domain);
        let refs=references.iter().take(20).map(String::as_str).chain(std::iter::once(mid)).collect::<Vec<_>>().join(" ");
        tx.execute("INSERT INTO outbox(id,message_id,recipient,subject,in_reply_to,refs,body,source,sender,created_at,kind)
            VALUES(?1,?2,?3,?4,?5,?6,?7,'task',?8,strftime('%s','now'),'error')",
            params![Uuid::new_v4().to_string(),reply_mid,recipient,
                fmt::reply_subject(subject),mid,refs,fmt::envelope("error",&reply_mid,error),mailbox])?;
        tx.commit()?;
        Ok(true)
    }
    pub fn queue_reply(&self, session_id: &str, mailbox: &str, body: &str, kind: &str) -> Result<()> {
        anyhow::ensure!(fmt::KINDS.contains(&kind), "invalid outbound mail kind: {kind}");
        let mut c = self.lock()?;
        let tx = c.transaction()?;
        let session = tx.query_row(
            "SELECT id,owner,agent,project,agent_session,subject,root_mid,last_mid,cwd FROM sessions WHERE id=?1",
            [session_id], Self::map_session)?;
        let domain = mailbox.split('@').nth(1).unwrap_or("localhost");
        let mid = format!("<ema-{}@{}>",Uuid::new_v4(), domain);
        let refs = if session.root_mid == session.last_mid { session.root_mid.clone() }
            else { format!("{} {}",session.root_mid,session.last_mid) };
        tx.execute("INSERT INTO outbox(id,message_id,recipient,subject,in_reply_to,refs,body,sender,created_at,kind)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,strftime('%s','now'),?9)",
            params![Uuid::new_v4().to_string(),mid,session.owner,fmt::reply_subject(&session.subject),
                session.last_mid, refs,fmt::envelope(kind,&mid,body),mailbox,kind])?;
        tx.execute("INSERT INTO messages(message_id,session_id,direction,created_at)
            VALUES(?1,?2,'out',strftime('%s','now'))",params![mid,session_id])?;
        tx.execute("UPDATE sessions SET last_mid=?1 WHERE id=?2",params![mid,session_id])?;
        tx.commit()?;
        Ok(())
    }
    pub fn is_ema_agent_session(&self, agent:&str, native_id:&str) -> Result<bool> {
        let agent=if agent=="claude" {"cc"} else {agent};
        Ok(self.lock()?.query_row(
            // ponytail: a short grace window catches asynchronously delivered hooks;
            // native run IDs can replace it if all providers expose them.
            "SELECT EXISTS(SELECT 1 FROM sessions s WHERE s.agent=?1 AND s.agent_session=?2
              AND (NOT EXISTS(SELECT 1 FROM outbox o WHERE o.message_id=s.root_mid AND o.source='hook')
                OR EXISTS(SELECT 1 FROM jobs j WHERE j.session_id=s.id AND
                  (j.state='running' OR (j.state IN ('completed','failed') AND j.completed_at>=strftime('%s','now')-10)))))",
            params![agent,native_id],|r|r.get::<_,i64>(0))? != 0)
    }
    pub fn queue_hook_report(&self, fingerprint:&str, recipient:&str,
        subject:&str, body:&str, agent:&str, native_session:&str, cwd:&str) -> Result<bool> {
        if recipient.trim().is_empty() { return Ok(false); }
        let mut c=self.lock()?;
        let tx=c.transaction()?;
        let inserted=tx.execute(
            "INSERT OR IGNORE INTO hook_events(fingerprint,created_at) VALUES(?1,strftime('%s','now'))",
            [fingerprint])?;
        if inserted == 0 { tx.rollback()?;return Ok(false); }
        let domain=recipient.split('@').nth(1).unwrap_or("localhost");
        let message_id=format!("<ema-hook-{}@{}>",Uuid::new_v4(),domain);
        tx.execute("INSERT INTO outbox(id,message_id,recipient,subject,in_reply_to,refs,body,source,sender,created_at,kind)
            VALUES(?1,?2,?3,?4,'','',?5,'hook',?3,strftime('%s','now'),'hook')",
            params![Uuid::new_v4().to_string(),message_id,recipient,fmt::single_line(subject,100),fmt::envelope("hook",&message_id,body)])?;
        tx.execute("INSERT INTO hook_context(message_id,agent,native_session,cwd) VALUES(?1,?2,?3,?4)",
            params![message_id,agent,native_session,cwd])?;
        tx.commit()?;
        Ok(true)
    }
    pub fn pending_outbox(&self, include_task:bool) -> Result<Vec<Outbound>> {
        let c=self.lock()?;
        let mut st=c.prepare("SELECT id,message_id,recipient,subject,in_reply_to,refs,body,attempts,kind
            FROM outbox WHERE state='queued' AND next_attempt <= strftime('%s','now')
                AND (?1=1 OR source='hook')
            ORDER BY rowid LIMIT 20")?;
        let rows=st.query_map([include_task],|r| Ok(Outbound {id:r.get(0)?,message_id:r.get(1)?,
            recipient:r.get(2)?,subject:r.get(3)?,in_reply_to:r.get(4)?,
            references:r.get(5)?,body:r.get(6)?,attempts:r.get(7)?,kind:r.get(8)?}))?;
        let outgoing=rows.collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(outgoing)
    }
    pub fn queue_connection_test(&self, mailbox: &str, recipient: &str, language: &str) -> Result<Outbound> {
        let domain=mailbox.split('@').nth(1).unwrap_or("localhost");
        let mut outgoing=Outbound {
            id:Uuid::new_v4().to_string(), message_id:format!("<ema-test-{}@{}>",Uuid::new_v4(),domain),
            recipient:recipient.to_string(), subject:text(language,"[连接测试] 邮件发送验证","[Connection test] Email delivery check").into(),
            in_reply_to:String::new(), references:String::new(),
            body:fmt::render(language, text(language,"这是 EMA 邮件发送测试，未创建 Agent 任务。","This is an EMA delivery test. No Agent task was created."),
                &[(text(language,"发送时间","Sent at"), &chrono::Utc::now().to_rfc3339())],
                &[(text(language,"验证范围","Scope"),text(language,"收到此邮件可确认本次 SMTP 投递；不代表入站轮询或 Agent 执行已验证。","Receipt confirms this SMTP delivery only. Inbound polling and Agent execution are not verified."))],
                text(language,"无需回复；请在 EMA 查看发送记录。","No reply needed. Check delivery history in EMA."), &[]), attempts:0,
            kind:"test".to_string(),
        };
        outgoing.body=fmt::envelope("test",&outgoing.message_id,&outgoing.body);
        self.lock()?.execute("INSERT INTO outbox(id,message_id,recipient,subject,in_reply_to,refs,body,source,sender,state,created_at,kind)
            VALUES(?1,?2,?3,?4,'','',?5,'test',?6,'sending',strftime('%s','now'),'test')",
            params![outgoing.id,outgoing.message_id,outgoing.recipient,outgoing.subject,outgoing.body,mailbox])?;
        Ok(outgoing)
    }
    pub fn outbox_result(&self,id:&str,error:Option<&str>,attempts:i64,retry:bool,mailbox:&str)->Result<()> {
        let mut c=self.lock()?;
        let tx=c.transaction()?;
        if error.is_none() {
            tx.execute("UPDATE outbox SET state='sent',attempts=attempts+1,sent_at=strftime('%s','now'),sender=?2 WHERE id=?1",params![id,mailbox])?;
        } else if !retry {
            tx.execute("UPDATE outbox SET state='failed',attempts=attempts+1,last_error=?2,sender=?3 WHERE id=?1",params![id,error,mailbox])?;
        } else {
            let delay=60_i64.saturating_mul(1_i64 << attempts.min(6));
            tx.execute("UPDATE outbox SET attempts=attempts+1,
                next_attempt=strftime('%s','now')+?2,last_error=?3,sender=?4 WHERE id=?1",params![id,delay,error,mailbox])?;
        }
        tx.execute("INSERT INTO outbox_attempts(outbox_id,attempted_at,state,error) VALUES(?1,strftime('%s','now'),?2,?3)",
            params![id,if error.is_none() {"sent"} else {"failed"},error.unwrap_or("")])?;
        tx.commit()?;
        Ok(())
    }
    pub fn stats(&self)->Result<Stats> {
        let c=self.lock()?;
        Ok(Stats{
            hook_reports:c.query_row("SELECT COUNT(*) FROM hook_events",[],|r|r.get(0))?,
            hook_sent:c.query_row("SELECT COUNT(*) FROM outbox WHERE source='hook' AND state='sent'",[],|r|r.get(0))?,
            hook_pending:c.query_row("SELECT COUNT(*) FROM outbox WHERE source='hook' AND state='queued'",[],|r|r.get(0))?,
            total_sessions:c.query_row("SELECT COUNT(*) FROM sessions",[],|r|r.get(0))?,
            completed_jobs:c.query_row("SELECT COUNT(*) FROM jobs WHERE state='completed'",[],|r|r.get(0))?,
            failed_jobs:c.query_row("SELECT COUNT(*) FROM jobs WHERE state IN ('failed','interrupted')",[],|r|r.get(0))?,
            pending_jobs:c.query_row("SELECT COUNT(*) FROM jobs WHERE state='queued'",[],|r|r.get(0))?,
            running_jobs:c.query_row("SELECT COUNT(*) FROM jobs WHERE state='running'",[],|r|r.get(0))?,
            received_messages:c.query_row("SELECT COUNT(*) FROM messages WHERE direction='in'",[],|r|r.get(0))?,
            last_activity:c.query_row("SELECT MAX(time) FROM (SELECT created_at AS time FROM messages UNION ALL SELECT created_at AS time FROM hook_events)",[],|r|r.get(0))?,
        })
    }
}
