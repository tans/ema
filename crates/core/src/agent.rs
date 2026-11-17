use anyhow::{Context, Result};
use crate::email_format::{self as fmt, text};
use std::{process::Stdio, sync::Arc, time::{Duration, Instant}};
use tokio::{io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader}, process::Command};
use crate::{config::Settings, db::{Database, Job, Session}};

/// Detect the installed CLI when the first email creates a native session.
pub async fn is_available(executable: &str) -> bool {
    let mut command = Command::new(executable);
    command.arg("--version").kill_on_drop(true)
        .stdout(Stdio::null()).stderr(Stdio::null());
    matches!(tokio::time::timeout(Duration::from_secs(5), command.status()).await,
        Ok(Ok(status)) if status.success())
}

/// Execute the installed CLI directly; never send email content through a shell.
pub async fn execute(db: Arc<Database>, config: Settings, job: Job, session: Session) -> Result<String> {
    if session.agent == "alma" {
        return crate::alma::execute(db, &config, &job, &session).await;
    }
    if session.agent != "codex" {
        return crate::cli::execute(db,&config,&job,&session).await;
    }
    if config.workspace_write && session.project == "_agent_memory" {
        anyhow::bail!("write access requires an explicitly authorized project");
    }
    let cwd = if let Some(path)=session.cwd.as_ref() {
        let path=std::path::PathBuf::from(path).canonicalize()?;
        if !path.is_dir() { anyhow::bail!("session directory is no longer available"); }
        path
    } else { config.project(&session.project)? };
    let executable=crate::cli::resolve("codex",&config).context("Codex CLI command is unavailable")?;
    let mut cmd = Command::new(executable);
    cmd.arg("exec").arg("--json").arg("-C").arg(&cwd)
        .arg("--sandbox").arg(if config.workspace_write { "workspace-write" } else { "read-only" });
    // Mail-only sessions use an isolated directory rather than a Git checkout.
    // This skips repository detection while retaining the configured sandbox.
    if session.project == "_agent_memory" { cmd.arg("--skip-git-repo-check"); }
    if let Some(existing) = &session.agent_session {
        cmd.arg("resume").arg(existing);
    }
    // Avoid leaking credential environment fallbacks to an untrusted child process.
    for key in ["EMA_MAIL_PASSWORD", "EMA_ROUTER_API_KEY", "EMA_MAIL_ACCESS_CODE"] { cmd.env_remove(key); }
    cmd.arg("-").current_dir(&cwd).stdin(Stdio::piped())
        .stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.kill_on_drop(true);
    let mut child = cmd.spawn().context("failed to start Codex CLI")?;
    let mut stdin = child.stdin.take().context("missing child stdin")?;
    stdin.write_all(job.prompt.as_bytes()).await?;
    stdin.shutdown().await?;
    drop(stdin);
    let stdout = child.stdout.take().context("missing Codex stdout")?;
    let mut lines = BufReader::new(stdout).lines();
    let stderr = child.stderr.take().context("missing stderr")?;
    let stderr_handle = tokio::spawn(async move {
        let mut bytes = Vec::new();
        let _ = stderr.take(16_384).read_to_end(&mut bytes).await;
        String::from_utf8_lossy(&bytes).to_string()
    });
    let mut final_text = String::new();
    let mut last_event = Instant::now();
    let started = Instant::now();
    let mut reminders = config.progress_minutes.clone();
    reminders.sort_unstable();
    reminders.dedup();
    let mut next = 0usize;
    let mut last_hourly = reminders.last().copied().unwrap_or(60);
    let mut ticker = tokio::time::interval(Duration::from_secs(20));
    loop {
        tokio::select! {
            line = lines.next_line() => {
                match line? {
                    Some(raw) => {
                        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) {
                            let kind = value.get("type").and_then(|v|v.as_str()).unwrap_or("event");
                            last_event = Instant::now();
                            if kind == "thread.started" {
                                if let Some(id) = value.get("thread_id").and_then(|v|v.as_str()) {
                                    db.set_agent_session(&session.id, id)?;
                                }
                            }
                            if kind == "item.completed" {
                                if let Some(item) = value.get("item") {
                                    if item.get("type").and_then(|v|v.as_str()) == Some("agent_message") {
                                        if let Some(text) = item.get("text").and_then(|v|v.as_str()) {
                                            final_text = fmt::excerpt(text, fmt::RESULT_LIMIT, &config.language);
                                        }
                                    }
                                    db.event(&job.id, kind,
                                        item.get("type").and_then(|v|v.as_str()).unwrap_or("item"))?;
                                }
                            }
                            if kind == "turn.failed" {
                                db.event(&job.id, kind, "Codex turn failed")?;
                            }
                        }
                    }
                    None => break,
                }
            }
            _ = ticker.tick() => {
                let minutes = started.elapsed().as_secs() / 60;
                let due = if next < reminders.len() && minutes >= reminders[next] {
                    next += 1;
                    true
                } else if next >= reminders.len() && minutes >= last_hourly + 60 {
                    last_hourly += 60;
                    true
                } else { false };
                if due {
                    let quiet = last_event.elapsed().as_secs();
                    let observed = if quiet > 600 {
                        format!("{} {}", text(&config.language,"未观察到新活动的分钟数：","Minutes since last activity:"), quiet / 60)
                    } else { format!("{} {}", text(&config.language,"距最近活动的秒数：","Seconds since last activity:"), quiet) };
                    let current = db.session(&session.id)?;
                    let summary = format!("{} {minutes} {}", text(&config.language,"任务已运行","Task has been running for"), text(&config.language,"分钟，Agent 进程仍在运行。","minutes; the Agent process is still running."));
                    let text = fmt::task_report(&config.language,&current,&job.id,&summary,text(&config.language,"运行中","Running"),
                        &[(text(&config.language,"观察信息","Observed activity"), &observed)],
                        text(&config.language,"请等待结果邮件；此通知不代表任务完成。可直接回复补充要求。","Wait for the result email. This update does not confirm completion. Reply with additional instructions if needed."));
                    db.queue_reply(&session.id, &config.mail.address, &text, "progress")?;
                }
            }
        }
    }
    let exit = child.wait().await?;
    let errors = stderr_handle.await.unwrap_or_default();
    if !exit.success() { anyhow::bail!("Codex failed: {}. {}",exit,errors.chars().take(1000).collect::<String>()); }
    if final_text.is_empty() { final_text = text(&config.language,"本轮已结束，但未获取最终回复。请检查本地日志。","This round ended without a final response. Check local logs.").into(); }
    Ok(final_text)
}
