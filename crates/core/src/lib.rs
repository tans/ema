pub mod agent;
pub mod config;
pub mod db;
pub mod mail;
pub mod hooks;
#[cfg(unix)]
pub mod hooks_extra;
pub mod cli;
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
        let a = self.clone();
        tokio::spawn(async move { a.hook_loop().await; });
        tokio::spawn(async move { self.job_loop().await; });
    }
    async fn hook_loop(self: Arc<Self>) {
        let mut ticks=0_u64;
        loop {
            if ticks % 12 == 0 {
                if let Err(err)=hooks::install(&self.dir) {
                    tracing::warn!("hook sync failed: {err:#}");
                }
            }
            let settings=self.settings();
            if let Err(err)=hooks::tick(&self.dir,&settings,&self.database) {
                tracing::warn!("hook collector failed: {err:#}");
            }
            ticks=ticks.wrapping_add(1);
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
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
        let (incoming, fetched_watermark, current_validity) = tokio::task::spawn_blocking(move || mail::receive(&mail_config, &pass, last, validity)).await??;
        let mut watermark = if validity == current_validity { last } else { 0 };
        let mut failed=false;
        for msg in incoming {
            let uid=msg.uid;
            match self.on_message(&config, msg).await {
                Ok(()) => { watermark=uid; self.database.set_cursor(&config.mail.address, watermark, current_validity)?; },
                Err(error) => { tracing::warn!(uid, "mail processing retry: {error:#}"); failed=true; break; },
            }
        }
        // Cursor consumes ignored malformed messages too, but never skips a failed task.
        if !failed { self.database.set_cursor(&config.mail.address, watermark.max(fetched_watermark), current_validity)?; }
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
        // Every configured prefix is routable. Never silently drop an unavailable
        // adapter: queue a single human-readable error in the original email thread.
        if !cli::runner_supported(&agent) {
            self.database.queue_route_error(
                &incoming.message_id,&incoming.sender,&cfg.mail.address,&incoming.subject,
                &format!("EMA could not start {agent}: this Agent does not yet have an email execution adapter. No task was executed."))?;
            return Ok(());
        }
        let Some(executable)=cli::resolve(&agent,cfg) else {
            self.database.queue_route_error(
                &incoming.message_id,&incoming.sender,&cfg.mail.address,&incoming.subject,
                &format!("EMA could not start {agent}: the Agent CLI was not found on this machine. Install it and retry; no task was executed."))?;
            return Ok(());
        };
        if !agent::is_available(&executable.to_string_lossy()).await {
            self.database.queue_route_error(
                &incoming.message_id,&incoming.sender,&cfg.mail.address,&incoming.subject,
                &format!("EMA could not start {agent}: the CLI cannot run its version check. Verify installation and retry; no task was executed."))?;
            return Ok(());
        }
        let resolution=if let Some(ref session)=previous {
            if let Some(ref cwd)=session.cwd {
                workspace::Resolution::Found(cwd.clone())
            } else if session.project != "_agent_memory" {
                workspace::Resolution::Found(cfg.project(&session.project)?.to_string_lossy().into_owned())
            } else { workspace::resolve(&self.dir,&self.database,cfg,&incoming.subject,&body,&session.id)? }
        } else { workspace::resolve(&self.dir,&self.database,cfg,&incoming.subject,&body,&incoming.message_id)? };
        let cwd=match &resolution { workspace::Resolution::Found(path)=>Some(path.as_str()), workspace::Resolution::Clarify=>None };
        let original = if let Some(ref session)=previous { self.database.pending_prompt(&session.id)? } else { None };
        let prompt = if let Some(ref initial)=original {
            format!("{initial}\n\nDirectory clarification from sender:\n{prompt}")
        } else { prompt };
        if let (Some(session),Some(path))=(previous.as_ref(),cwd) {
            self.database.update_cwd(&session.id,path)?;
            self.database.resolve_pending(&session.id)?;
        }
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
            if !cfg.mail.address.trim().is_empty() && !cfg.mail.smtp_host.trim().is_empty() {
                if let Err(err) = self.flush_outbox(cfg).await {
                    tracing::warn!("outbox failed: {err:#}");
                }
            }
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    }
    async fn flush_outbox(&self, cfg: Settings) -> Result<()> {
        let pending = self.database.pending_outbox(cfg.enabled)?;
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
