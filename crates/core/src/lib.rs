pub mod agent;
pub mod config;
pub mod db;
pub mod mail;
pub mod router;
pub mod workspace;

use anyhow::Result;
use config::Settings;
use db::{Database, Stats};
use std::{path::PathBuf, sync::{Arc, RwLock}, time::Duration};

pub struct Gateway {
    dir: PathBuf,
    pub database: Arc<Database>,
    settings: RwLock<Settings>,
}
impl Gateway {
    pub fn open(dir: PathBuf) -> Result<Arc<Self>> {
        let cfg = Settings::read(&dir)?;
        let db = Arc::new(Database::open(&dir.join("ema.db"))?);
        Ok(Arc::new(Self { dir, database: db, settings: RwLock::new(cfg) }))
    }
    pub fn settings(&self) -> Settings {
        self.settings.read().expect("settings lock").clone()
    }
    pub fn save(&self, new: Settings) -> Result<()> {
        new.write(&self.dir)?;
        *self.settings.write().expect("settings lock") = new;
        Ok(())
    }
    pub fn stats(&self) -> Result<Stats> { self.database.stats() }
    pub async fn run(self: Arc<Self>) {
        let a = self.clone();
        tokio::spawn(async move { a.mail_loop().await; });
        let a = self.clone();
        tokio::spawn(async move { a.outbox_loop().await; });
        tokio::spawn(async move { self.job_loop().await; });
    }
    async fn mail_loop(self: Arc<Self>) {
        loop {
            let cfg = self.settings();
            if cfg.enabled {
                if let Err(error) = self.poll_once(cfg.clone()).await {
                    tracing::warn!("mail poll failed: {error:#}");
                }
            }
            tokio::time::sleep(Duration::from_secs(cfg.poll_seconds.max(15))).await;
        }
    }
    async fn poll_once(&self, config: Settings) -> Result<()> {
        let password = config::secret("mail")?;
        let mail_config = config.mail.clone();
        let pass = password.clone();
        let (last,validity) = self.database.cursor(&mail_config.address)?;
        let (incoming, _watermark, current_validity) = tokio::task::spawn_blocking(move || mail::receive(&mail_config, &pass, last, validity)).await??;
        let mut watermark = if validity == current_validity { last } else { 0 };
        for msg in incoming {
            let uid=msg.uid;
            match self.on_message(&config, msg).await {
                Ok(()) => { watermark=uid; self.database.set_cursor(&config.mail.address, watermark, current_validity)?; },
                Err(error) => { tracing::warn!(uid, "mail processing retry: {error:#}"); break; },
            }
        }
        // UID cursor never modifies user read/unread state.
        if validity != current_validity && watermark == 0 { self.database.set_cursor(&config.mail.address, 0, current_validity)?; }
        Ok(())
    }
    async fn on_message(&self, cfg: &Settings, incoming: mail::Incoming) -> Result<()> {
        if incoming.automated || incoming.sender == cfg.mail.address.to_ascii_lowercase() {
            return Ok(());
        }
        if !cfg.is_allowed(&incoming.sender) {
            tracing::warn!("mail rejected: unauthorized sender");
            return Ok(());
        }
        if incoming.body.trim().is_empty() { return Ok(()); }
        let mut body = incoming.body.trim().to_string();
        // Workspace-write mode requires an additional out-of-band secret per message.
        // Remove the secret before handing user input to any model or persistence layer.
        if cfg.workspace_write {
            let first = body.lines().next().unwrap_or("");
            let provided = first.strip_prefix("EMA-Code: ").unwrap_or("");
            let expected = config::secret("access")?;
            if !constant_time_eq(provided.as_bytes(),expected.as_bytes()) {
                tracing::warn!("write-mode mail rejected: access code required");
                return Ok(());
            }
            body = body.lines().skip(1).collect::<Vec<_>>().join("\n");
            if body.trim().is_empty() { return Ok(()); }
        }
        let previous = self.database.find_by_references(&incoming.references)?;
        if let Some(ref session) = previous {
            if session.owner != incoming.sender {
                tracing::warn!("mail rejected: session owner mismatch");
                return Ok(());
            }
        }
        let (agent,project,prompt) = if let Some(ref session)=previous {
            (session.agent.clone(),session.project.clone(),body.clone())
        } else {
            let Some(decision)=router::route(cfg,&incoming.subject,&body) else { return Ok(()); };
            (decision.agent,decision.project,decision.task)
        };
        if agent != "codex" {
            tracing::warn!(agent, "agent execution adapter unavailable");
            return Ok(());
        }
        if !agent::is_available(&cfg.codex_command).await {
            anyhow::bail!("Codex CLI unavailable; install and authenticate Codex");
        }
        let resolution=if let Some(ref session)=previous {
            if let Some(ref cwd)=session.cwd {
                workspace::Resolution::Found(cwd.clone())
            } else { workspace::resolve(&self.dir,&self.database,cfg,&incoming.subject,&body,&session.id)? }
        } else { workspace::resolve(&self.dir,&self.database,cfg,&incoming.subject,&body,&incoming.message_id)? };
        let cwd=match &resolution { workspace::Resolution::Found(path)=>Some(path.as_str()), workspace::Resolution::Clarify=>None };
        if let (Some(session),Some(path))=(previous.as_ref(),cwd) { self.database.update_cwd(&session.id,path)?; }
        let enqueue=cwd.is_some();
        let stored=self.database.store_inbound(&incoming.message_id,&incoming.sender,previous.as_ref(),&agent,
            &project,&incoming.subject,&prompt,cwd,enqueue)?;
        if let Some(id)=stored {
            let text=if enqueue {format!("Task queued.\nAgent: {agent}\n\nReply to continue this session.")}
                else { "Which local project directory should this task use? Reply with a known project name; no task has started.".to_string() };
            self.database.queue_reply(&id,&cfg.mail.address,&text)?;
        }
        Ok(())
    }
    async fn job_loop(self: Arc<Self>) {
        loop {
            let cfg = self.settings();
            if cfg.enabled {
                if let Err(err) = self.work_once(cfg).await {
                    tracing::error!("task runner error: {err:#}");
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }
    async fn work_once(&self, cfg: Settings) -> Result<()> {
        let Some(job) = self.database.claim()? else { return Ok(()); };
        let session = self.database.session(&job.session_id)?;
        let result = agent::execute(self.database.clone(), cfg.clone(), job.clone(), session).await;
        let (success, report) = match result {
            Ok(message) => (true, message),
            Err(err) => (false, format!("Task failed: {err:#}")),
        };
        self.database.finish(&job.id, success, &report)?;
        self.database.queue_reply(&job.session_id, &cfg.mail.address,
            &format!("{}\n\n{}", if success {"Task completed"} else {"Task failed"}, report))?;
        Ok(())
    }
    async fn outbox_loop(self: Arc<Self>) {
        loop {
            let cfg = self.settings();
            if cfg.enabled {
                if let Err(err) = self.flush_outbox(cfg).await {
                    tracing::warn!("outbox failed: {err:#}");
                }
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }
    async fn flush_outbox(&self, cfg: Settings) -> Result<()> {
        let pending = self.database.pending_outbox()?;
        if pending.is_empty() { return Ok(()); }
        let pass = config::secret("mail")?;
        for outgoing in pending {
            let mail_config = cfg.mail.clone();
            let password = pass.clone();
            let id = outgoing.id.clone();
            let attempts = outgoing.attempts;
            let result = tokio::task::spawn_blocking(move ||
                mail::send(&mail_config, &password, &outgoing)).await?;
            self.database.outbox_result(&id, result.is_ok(), attempts)?;
            if let Err(err) = result { tracing::warn!("smtp send failed: {err:#}"); }
        }
        Ok(())
    }
}
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.is_empty() || a.len() != b.len() { return false; }
    let mut diff = 0_u8;
    for (x,y) in a.iter().zip(b) { diff |= x ^ y; }
    diff == 0
}
