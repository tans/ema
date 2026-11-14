use anyhow::{Context, Result};
use std::{process::Stdio, sync::Arc, time::{Duration, Instant}};
use tokio::{io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader}, process::Command};
use crate::{config::Settings, db::{Database, Job, Session}};

/// Execute the installed CLI directly; never send email content through a shell.
pub async fn execute(db: Arc<Database>, config: Settings, job: Job, session: Session) -> Result<String> {
    let cwd = config.project(&session.project)?;
    let mut cmd = Command::new(&config.codex_command);
    cmd.arg("exec").arg("--json").arg("-C").arg(&cwd)
        .arg("--sandbox").arg(if config.workspace_write { "workspace-write" } else { "read-only" });
    if let Some(existing) = &session.agent_session {
        cmd.arg("resume").arg(existing);
    }
    // Avoid leaking credential environment fallbacks to an untrusted child process.
    for key in ["EMA_MAIL_PASSWORD", "EMA_ROUTER_API_KEY", "EMA_MAIL_ACCESS_CODE"] { cmd.env_remove(key); }
    cmd.arg("-").current_dir(&cwd).stdin(Stdio::piped())
        .stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().context("failed to start Codex CLI")?;
    let mut stdin = child.stdin.take().context("missing child stdin")?;
    stdin.write_all(job.prompt.as_bytes()).await?;
    stdin.shutdown().await?;
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
                                            final_text = text.chars().take(12_000).collect();
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
                        format!("Agent process still running; no new activity for {} minutes.", quiet / 60)
                    } else { format!("Last observed Agent activity {} seconds ago.", quiet) };
                    let text = format!("Task still running ({} minutes).\n{}\n\nThis is an automated status update, not a completion report.", minutes, observed);
                    db.queue_reply(&session.id, &config.mail.address, &text)?;
                }
            }
        }
    }
    let exit = child.wait().await?;
    let errors = stderr_handle.await.unwrap_or_default();
    if !exit.success() { anyhow::bail!("Codex failed: {}. {}",exit,errors.chars().take(1000).collect::<String>()); }
    if final_text.is_empty() { final_text = "Codex finished without a final text response. Check local logs.".into(); }
    Ok(final_text)
}
