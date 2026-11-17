mod local_reply;
pub use local_reply::ReplyContext;
pub mod agent;
pub mod config;
pub mod db;
pub mod mail;
pub mod email_format;
pub mod hooks;
#[cfg(unix)]
pub mod hooks_alma;
pub mod cli;
pub mod router;
pub mod workspace;
pub mod alma;

use anyhow::Result;
use config::Settings;
use email_format::{self as fmt, text};
use db::{Database, Stats};
use std::{path::PathBuf, sync::{Arc, RwLock}, time::Duration};

pub struct Gateway {
    dir: PathBuf,
    pub database: Arc<Database>,
    settings: RwLock<Settings>,
    outbox_ready: tokio::sync::Notify,
    mail_poll_lock: tokio::sync::Mutex<()>,
    activity: Arc<tokio::sync::RwLock<()>>,
}
impl Gateway {
    pub fn open(dir: PathBuf) -> Result<Arc<Self>> {
        let cfg = Settings::read(&dir)?;
        let db = Arc::new(Database::open(&dir.join("ema.db"))?);
        Ok(Arc::new(Self { dir, database: db, settings: RwLock::new(cfg), outbox_ready: tokio::sync::Notify::new(), mail_poll_lock: tokio::sync::Mutex::new(()), activity: Arc::new(tokio::sync::RwLock::new(())) }))
    }
    /// Every operation that can receive, send or execute mail holds a shared guard.
    pub fn begin_activity(&self) -> Result<tokio::sync::OwnedRwLockReadGuard<()>> {
        self.activity.clone().try_read_owned().map_err(|_| anyhow::anyhow!("{}", text(&self.settings().language,
            "正在安装更新，请稍后重试。", "An update is being installed. Try again later.")))
    }
    /// Fail immediately when busy; holding this guard closes the check/start race.
    pub fn prepare_update(&self) -> Result<tokio::sync::OwnedRwLockWriteGuard<()>> {
        self.activity.clone().try_write_owned().map_err(|_| anyhow::anyhow!("{}", text(&self.settings().language,
            "邮件或 Agent 任务正在运行，请完成后重试安装。", "Mail or Agent work is running. Retry installation when it finishes.")))
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
            if let Ok(_activity) = self.begin_activity() {
                if ticks % 12 == 0 {
                    if let Err(err)=hooks::install(&self.dir) {
                        tracing::warn!("hook sync failed: {err:#}");
                    }
                }
                let settings=self.settings();
                if let Err(err)=hooks::tick(&self.dir,&settings,&self.database) {
                    tracing::warn!("hook collector failed: {err:#}");
                }
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
    /// Manual receipt uses exactly the same pipeline as the scheduled poll.
    pub async fn receive_mail_now(&self) -> Result<()> {
        let cfg = self.settings();
        anyhow::ensure!(cfg.enabled, "{}", text(&cfg.language,
            "收件已暂停，请先在设置中启用接收邮件任务。",
            "Mail receiving is paused. Enable Receive email tasks in Settings first."));
        self.poll_once(cfg).await
    }
    async fn poll_once(&self, config: Settings) -> Result<()> {
        let _activity = self.begin_activity()?;
        // Hold across cursor read, fetch, classification and watermark update.
        let _guard = self.mail_poll_lock.lock().await;
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
        anyhow::ensure!(!failed, "{}", text(&config.language,
            "部分邮件处理失败，将在下次收件时重试。", "Some mail could not be processed and will be retried on the next check."));
        Ok(())
    }
    async fn on_message(&self, cfg: &Settings, incoming: mail::Incoming) -> Result<&'static str> {
        // Self-identification must run before automated-mail filtering.
        let marker_mid = fmt::marker_id(&incoming.body);
        let own_id = self.database.is_outbound(&incoming.message_id)?;
        if own_id || incoming.ema_kind.is_some() || incoming.ema_message_id.is_some() || marker_mid.is_some() {
            if let Some(mid) = incoming.ema_message_id.as_deref().or(marker_mid) {
                if self.database.is_outbound(mid)? { self.database.record_alias(&incoming.message_id, mid)?; }
            }
            self.database.set_inbound_kind(&incoming.message_id, "echo")?;
            return Ok("echo");
        }
        if incoming.automated { return Ok("automated"); }
        if !cfg.is_allowed(&incoming.sender) {
            tracing::warn!("mail rejected: unauthorized sender");
            return Ok("unauthorized");
        }
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
        }
        let previous = self.database.find_by_references(&incoming.references)?;
        let mut hook=self.database.find_hook_report(&incoming.references)?;
        if hook.is_none() {
            if let Some(session)=previous.as_ref() {
                hook=self.database.find_hook_report(&[session.root_mid.clone()])?;
            }
        }
        // The allowlisted mailbox owner may send new tasks to themselves.
        // EMA-generated mail was already excluded by its headers/body marker above.
        if previous.is_some() || hook.is_some() {
            body=reply_text(&body);
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
        // Every configured prefix is routable. There is no static support
        // whitelist: resolution below (executable present on this machine)
        // and the availability probe are the real gates. If execution later
        // fails, the caller receives the actual error instead of a refusal.
        if agent!="alma" {
        let Some(executable)=cli::resolve(&agent,cfg) else {
            self.database.queue_route_error(
                &incoming.message_id,&incoming.sender,&cfg.mail.address,&incoming.subject,
                &fmt::route_error(&cfg.language, &agent, previous.as_ref().and_then(|s|s.agent_session.as_deref()).unwrap_or(""), &body,
                    "AGENT_NOT_INSTALLED", text(&cfg.language,"本机未找到 Agent 命令。","The Agent CLI was not found on this machine."),
                    text(&cfg.language,"安装或配置 Agent 后，重新发送任务。","Install or configure the Agent, then resend the task.")),&incoming.references)?;
            self.outbox_ready.notify_one(); return Ok("not_installed");
        };
        if !agent::is_available(&executable.to_string_lossy()).await {
            self.database.queue_route_error(
                &incoming.message_id,&incoming.sender,&cfg.mail.address,&incoming.subject,
                &fmt::route_error(&cfg.language, &agent, "", &body,
                    "AGENT_UNAVAILABLE", text(&cfg.language,"Agent 命令未通过可用性检查。","The Agent CLI failed its availability check."),
                    text(&cfg.language,"检查本机 Agent 配置后，重新发送任务。","Check the local Agent configuration, then resend the task.")),&incoming.references)?;
            self.outbox_ready.notify_one(); return Ok("unavailable_runner");
        }
        }
        if hook.as_ref().is_some_and(|report|report.native_session.is_empty()) {
            self.database.queue_route_error(&incoming.message_id,&incoming.sender,&cfg.mail.address,&incoming.subject,
                &fmt::route_error(&cfg.language, &agent, "", &body,
                    "SESSION_MISSING", text(&cfg.language,"原报告缺少 Agent 会话 ID，无法续接。","The original report has no Agent session ID and cannot be resumed."),
                    text(&cfg.language,"请在原应用继续，或使用 Agent 前缀发起新任务。","Continue in the original application or send a new task with an Agent prefix.")),&incoming.references)?;
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
                // Legacy reports only: current reports carry cwd in hook_context.
                let legacy = report.body.lines().find_map(|line|line.strip_prefix("Project: ").or_else(||line.strip_prefix("项目：")));
                let project = report.cwd.as_deref().and_then(|p|std::path::Path::new(p).file_name()).and_then(|p|p.to_str()).or(legacy).unwrap_or("unknown");
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
            &project,&incoming.subject,&prompt,cwd,enqueue,&cfg.mail.address,&body,hook.as_ref(),&cfg.language)?;
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
        let _activity = self.begin_activity()?;
        let Some(job) = self.database.claim()? else { return Ok(()); };
        let session = self.database.session(&job.session_id)?;
        let result = agent::execute(self.database.clone(), cfg.clone(), job.clone(), session.clone()).await;
        let (success, report) = match result {
            Ok(message) => (true, message),
            Err(err) => (false, format!("{err:#}")),
        };
        self.database.finish(&job.id, success, &report)?;
        // Refresh the native session ID learned while running.
        let session = self.database.session(&job.session_id)?;
        let summary = if success { text(&cfg.language,"本轮执行结束，请查看 Agent 回复。","This execution round has ended. Review the Agent response.") }
            else { text(&cfg.language,"本轮执行失败，请查看原因。","This execution round failed. Review the reason.") };
        let body = fmt::task_report(&cfg.language, &session, &job.id, summary,
            if success { text(&cfg.language,"完成","Completed") } else { text(&cfg.language,"失败","Failed") },
            &[(text(&cfg.language,"结果 / 原因","Result / reason"), &fmt::excerpt(&report,fmt::RESULT_LIMIT,&cfg.language))],
            text(&cfg.language,"请检查结果；执行结束不代表所有目标均已完成。可直接回复继续此会话。","Review the result; execution ending does not confirm every goal is achieved. Reply to continue this session."));
        self.database.queue_reply(&job.session_id, &cfg.mail.address, &body, "result")?;
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
        let _activity = self.begin_activity()?;
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

#[cfg(test)]
mod mail_regression_tests {
    use super::*;

    fn gateway() -> (Arc<Gateway>, PathBuf) {
        let dir = std::env::temp_dir().join(format!("ema-mail-regression-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = Settings::default();
        cfg.mail.address = "owner@example.com".into();
        cfg.allowlist = vec![cfg.mail.address.clone()];
        cfg.codex_command = dir.join("missing-codex").to_string_lossy().into_owned();
        cfg.write(&dir).unwrap();
        (Gateway::open(dir.clone()).unwrap(), dir)
    }
    fn incoming(body: &str) -> mail::Incoming {
        mail::Incoming { uid: 1, message_id: "<user-task@example.com>".into(),
            sender: "owner@example.com".into(), subject: "codex Check mail".into(),
            body: body.into(), references: vec![], automated: false, ema_kind: None, ema_message_id: None }
    }
    #[tokio::test]
    async fn updater_excludes_active_work_and_blocks_new_work_until_released() {
        let (gateway, dir) = gateway();
        let activity = gateway.begin_activity().unwrap();
        assert!(gateway.prepare_update().is_err(), "must not interrupt active mail work");
        drop(activity);
        let installation = gateway.prepare_update().unwrap();
        assert!(gateway.begin_activity().is_err());
        assert!(gateway.prepare_update().is_err());
        let cfg = gateway.settings();
        assert!(gateway.poll_once(cfg.clone()).await.is_err());
        assert!(gateway.work_once(cfg.clone()).await.is_err());
        assert!(gateway.flush_outbox(cfg).await.is_err());
        assert_eq!(gateway.reply_to_mail("missing", "invalid", "test").await.unwrap_err().to_string(),
            gateway.begin_activity().unwrap_err().to_string());
        drop(installation);
        assert!(gateway.begin_activity().is_ok(), "failed/cancelled install must allow work again");
        drop(gateway);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[tokio::test]
    async fn manual_receive_does_not_bypass_pause() {
        let (gateway, dir) = gateway();
        let error = gateway.receive_mail_now().await.unwrap_err();
        assert!(error.to_string().contains("paused") || error.to_string().contains("暂停"));
        assert!(gateway.database.pending_outbox(true).unwrap().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[tokio::test]
    async fn allowlisted_self_mail_reaches_codex_routing() {
        let (gateway, dir) = gateway();
        assert_eq!(gateway.on_message(&gateway.settings(), incoming("Only read.")).await.unwrap(), "not_installed");
        let outgoing = gateway.database.pending_outbox(true).unwrap();
        assert_eq!(outgoing.len(), 1);
        assert_eq!(outgoing[0].kind, "error");
        assert!(outgoing[0].body.contains("AGENT_NOT_INSTALLED"));
        assert_eq!(gateway.database.stats().unwrap().total_sessions, 0);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[tokio::test]
    async fn empty_body_task_routes_like_normal() {
        let (gateway, dir) = gateway();
        let message = incoming("");
        let fetched = mail::FetchedMail { uid: 1, message_id: message.message_id.clone(),
            sender: message.sender.clone(), recipient: message.sender.clone(), cc: String::new(),
            references: String::new(), original_date: String::new(), subject: message.subject.clone(),
            body: String::new(), archive_note: String::new(), message: None, problem: "" };
        gateway.database.record_incoming(&message.sender, 1, &fetched).unwrap();
        // An empty body is not a reason to refuse: the subject still carries the
        // instruction, so routing proceeds exactly like any other task.
        let status = gateway.on_message(&gateway.settings(), message).await.unwrap();
        assert_eq!(status, "not_installed");
        let outgoing = gateway.database.pending_outbox(true).unwrap();
        assert_eq!(outgoing.len(), 1);
        assert_eq!(outgoing[0].kind, "error");
        assert!(outgoing[0].body.contains("AGENT_NOT_INSTALLED"));
        assert!(!outgoing[0].body.contains("EMPTY_BODY"));
        assert_eq!(gateway.database.stats().unwrap().total_sessions, 0);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
