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
pub mod alma;

use anyhow::Result;
use config::Settings;
use db::{Database, Stats};
use std::{path::PathBuf, sync::{Arc, RwLock}, time::Duration};

pub struct Gateway {
    dir: PathBuf,
    pub database: Arc<Database>,
    settings: RwLock<Settings>,
    outbox_ready: tokio::sync::Notify,
}
impl Gateway {
    pub fn open(dir: PathBuf) -> Result<Arc<Self>> {
        let cfg = Settings::read(&dir)?;
        let db = Arc::new(Database::open(&dir.join("ema.db"))?);
        Ok(Arc::new(Self { dir, database: db, settings: RwLock::new(cfg), outbox_ready: tokio::sync::Notify::new() }))
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
        for fetched in &incoming {
            self.database.record_incoming(&config.mail.address, current_validity, fetched)?;
        }
        for fetched in incoming {
            let uid=fetched.uid;
            let result = match fetched.message {
                Some(message) => self.on_message(&config, message).await,
                None => Ok(fetched.problem),
            };
            match result {
                Ok(status) => {
                    self.database.incoming_result(&config.mail.address,current_validity,uid,status,"")?;
                    watermark=uid;
                    self.database.set_cursor(&config.mail.address, watermark, current_validity)?;
                },
                Err(error) => {
                    self.database.incoming_result(&config.mail.address,current_validity,uid,"retry","")?;
                    tracing::warn!(uid, "mail processing retry: {error:#}"); failed=true; break;
                },
            }
        }
        // Advance only after the fetched message and its processing result are stored.
        if !failed { self.database.set_cursor(&config.mail.address, watermark.max(fetched_watermark), current_validity)?; }
        Ok(())
    }
    async fn on_message(&self, cfg: &Settings, incoming: mail::Incoming) -> Result<&'static str> {
        if incoming.automated || self.database.is_outbound(&incoming.message_id)? {
            return Ok("automated");
        }
        if !cfg.is_allowed(&incoming.sender) {
            tracing::warn!("mail rejected: unauthorized sender");
            return Ok("unauthorized");
        }
        if incoming.body.trim().is_empty() { return Ok("empty"); }
        let mut body = incoming.body.trim().to_string();
        // Workspace-write mode requires an additional out-of-band secret per message.
        // Remove the secret before handing user input to any model or persistence layer.
        if cfg.workspace_write {
            let first = body.lines().next().unwrap_or("");
            let provided = first.strip_prefix("EMA-Code: ").unwrap_or("");
            let expected = config::secret("access")?;
            if !constant_time_eq(provided.as_bytes(),expected.as_bytes()) {
                tracing::warn!("write-mode mail rejected: access code required");
                return Ok("access_denied");
            }
            body = body.lines().skip(1).collect::<Vec<_>>().join("\n");
            if body.trim().is_empty() { return Ok("empty"); }
        }
        let previous = self.database.find_by_references(&incoming.references)?;
        let mut hook=self.database.find_hook_report(&incoming.references)?;
        if hook.is_none() {
            if let Some(session)=previous.as_ref() {
                hook=self.database.find_hook_report(&[session.root_mid.clone()])?;
            }
        }
        // Only known threads may be continued from the gateway's own address.
        // Its generated messages are filtered by Message-ID above.
        if incoming.sender.eq_ignore_ascii_case(&cfg.mail.address) && previous.is_none() && hook.is_none() {
            return Ok("ordinary");
        }
        if previous.is_some() || hook.is_some() {
            body=reply_text(&body);
            if body.is_empty() { return Ok("empty"); }
        }
        if let Some(ref session) = previous {
            if session.owner != incoming.sender {
                tracing::warn!("mail rejected: session owner mismatch");
                return Ok("owner_mismatch");
            }
        }
        let (agent,project,prompt) = if let Some(ref session)=previous {
            (session.agent.clone(),session.project.clone(),body.clone())
        } else if let Some(ref report)=hook {
            let name=if report.agent=="claude" {"cc"} else {report.agent.as_str()};
            (name.to_string(),"_agent_memory".to_string(),body.clone())
        } else {
            let Some(decision)=router::route(cfg,&incoming.subject,&body) else { return Ok("ordinary"); };
            (decision.agent,decision.project,decision.task)
        };
        // Every configured prefix is routable. Never silently drop an unavailable
        // adapter: queue a single human-readable error in the original email thread.
        let supported=if hook.is_some() {cli::resume_supported(&agent)} else {cli::runner_supported(&agent)};
        if !supported && !(agent=="alma" && hook.is_some()) {
            self.database.queue_route_error(
                &incoming.message_id,&incoming.sender,&cfg.mail.address,&incoming.subject,
                &format!("Agent: {agent}\nSession: {}\n\nTask:\n{}\n\nEMA cannot resume this Agent yet. No task was executed.",
                    hook.as_ref().map(|r|r.native_session.as_str()).unwrap_or("unavailable"),body.chars().take(2000).collect::<String>()),&incoming.references)?;
            self.outbox_ready.notify_one();
            return Ok("unsupported");
        }
        if agent!="alma" {
        let Some(executable)=cli::resolve(&agent,cfg) else {
            self.database.queue_route_error(
                &incoming.message_id,&incoming.sender,&cfg.mail.address,&incoming.subject,
                &format!("Agent: {agent}\nSession: {}\n\nTask:\n{}\n\nThe Agent CLI was not found on this machine. No task was executed.",
                    hook.as_ref().map(|r|r.native_session.as_str()).or_else(||previous.as_ref().and_then(|s|s.agent_session.as_deref())).unwrap_or("unavailable"),body.chars().take(2000).collect::<String>()),&incoming.references)?;
            self.outbox_ready.notify_one(); return Ok("not_installed");
        };
        if !agent::is_available(&executable.to_string_lossy()).await {
            self.database.queue_route_error(
                &incoming.message_id,&incoming.sender,&cfg.mail.address,&incoming.subject,
                &format!("Agent: {agent}\nSession: {}\n\nTask:\n{}\n\nThe Agent CLI failed its version check. No task was executed.",
                    hook.as_ref().map(|r|r.native_session.as_str()).or_else(||previous.as_ref().and_then(|s|s.agent_session.as_deref())).unwrap_or("unavailable"),body.chars().take(2000).collect::<String>()),&incoming.references)?;
            self.outbox_ready.notify_one(); return Ok("unavailable_runner");
        }
        }
        if hook.as_ref().is_some_and(|report|report.native_session.is_empty()) {
            self.database.queue_route_error(&incoming.message_id,&incoming.sender,&cfg.mail.address,&incoming.subject,
                &format!("Agent: {agent}\nSession: unavailable\n\nTask:\n{}\n\nThe report has no session ID, so EMA cannot resume the original session. No task was executed.",body.chars().take(2000).collect::<String>()),&incoming.references)?;
            self.outbox_ready.notify_one();
            return Ok("missing_session");
        }
        let resolution=if agent=="alma" {
            workspace::Resolution::Clarify
        } else if let Some(ref session)=previous {
            if let Some(ref cwd)=session.cwd {
                workspace::Resolution::Found(cwd.clone())
            } else if session.project != "_agent_memory" {
                workspace::Resolution::Found(cfg.project(&session.project)?.to_string_lossy().into_owned())
            } else { workspace::resolve(&self.dir,&self.database,cfg,&incoming.subject,&body,&session.id)? }
        } else if let Some(ref report)=hook {
            if let Some(path)=report.cwd.as_deref().and_then(|p|std::fs::canonicalize(p).ok()).filter(|p|p.is_dir()) {
                workspace::Resolution::Found(path.to_string_lossy().into_owned())
            } else {
                let project=report.body.lines().find_map(|line|line.strip_prefix("Project: ").or_else(||line.strip_prefix("项目："))).unwrap_or("unknown");
                if project=="unknown" || project=="未提供" || project.trim().is_empty() { workspace::Resolution::Clarify }
                else { workspace::resolve(&self.dir,&self.database,cfg,"project",project,&incoming.message_id)? }
            }
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
        let enqueue=agent=="alma" || cwd.is_some();
        let stored=self.database.store_inbound(&incoming.message_id,&incoming.sender,previous.as_ref(),&agent,
            &project,&incoming.subject,&prompt,cwd,enqueue,&cfg.mail.address,&body,hook.as_ref())?;
        if stored.is_some() { self.outbox_ready.notify_one(); }
        Ok(if stored.is_none() { "duplicate" } else if enqueue { "queued" } else { "awaiting_directory" })
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
        self.outbox_ready.notify_one();
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
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(10)) => {},
                _ = self.outbox_ready.notified() => {},
            }
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
            let error=result.as_ref().err().map(|err|mail::redact_smtp_error(&format!("{err:#}"),&pass));
            self.database.outbox_result(&id,error.as_deref(),attempts,true,&cfg.mail.address)?;
            if let Some(err) = error { tracing::warn!("smtp send failed: {err}"); }
        }
        Ok(())
    }
}
fn reply_text(body: &str) -> String {
    body.lines().take_while(|line| {
        let s=line.trim();
        !(s.starts_with('>') || (s.starts_with('-') && (s.contains("Original Message") || s.contains("原始邮件")))
            || (s.starts_with("On ") && s.ends_with("wrote:"))
            || (s.starts_with('在') && (s.ends_with("写道:") || s.ends_with("写道："))))
    }).collect::<Vec<_>>().join("\n").trim().to_string()
}
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.is_empty() || a.len() != b.len() { return false; }
    let mut diff = 0_u8;
    for (x,y) in a.iter().zip(b) { diff |= x ^ y; }
    diff == 0
}
