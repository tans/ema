//! Continue an existing Alma thread using its local thread-sync protocol.
//! Fresh tasks (no existing thread yet) create a new thread through the local
//! API first, then continue it with the same protocol.
use anyhow::{bail, Context, Result};
use std::{path::PathBuf, process::Stdio, time::Duration, sync::Arc};
use tokio::{io::AsyncWriteExt, process::Command};
use crate::{config::Settings, db::{Database, Job, Session}};

const ALMA_API: &str = "127.0.0.1:23001";

/// Create a brand-new Alma thread via the local API and return its ID.
fn create_thread(title: &str) -> Result<String> {
    use std::io::{Read as _, Write as _};
    use std::net::TcpStream;
    let title = title.chars().filter(|c| !c.is_control()).take(40).collect::<String>();
    let body = serde_json::json!({ "title": title }).to_string();
    let mut stream = TcpStream::connect(ALMA_API).context("Alma local API is unreachable")?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
    write!(stream, "POST /api/threads HTTP/1.1\r\nHost: {ALMA_API}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(), body).ok();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).ok();
    let text = String::from_utf8_lossy(&bytes);
    let body = text.find("\r\n\r\n").map(|at| &text[at + 4..]).context("invalid Alma thread creation response")?;
    let value: serde_json::Value = serde_json::from_str(body).context("invalid Alma thread creation JSON")?;
    let id = value.get("id").and_then(|v| v.as_str()).context("Alma thread creation returned no ID")?;
    if id.is_empty() { bail!("Alma thread creation returned an empty ID"); }
    Ok(id.to_string())
}

fn runtime() -> Option<PathBuf> {
    let mut candidates=Vec::new();
    #[cfg(target_os="macos")]
    candidates.push(PathBuf::from("/Applications/Alma.app/Contents/Resources/alma-server/resources/bun/bun"));
    if let Some(home)=std::env::var_os("HOME").map(PathBuf::from) {
        #[cfg(target_os="macos")]
        candidates.push(home.join("Applications/Alma.app/Contents/Resources/alma-server/resources/bun/bun"));
        candidates.push(home.join(".bun/bin/bun"));
    }
    if let Some(path)=std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            candidates.push(dir.join(if cfg!(windows) {"bun.exe"} else {"bun"}));
            candidates.push(dir.join(if cfg!(windows) {"node.exe"} else {"node"}));
        }
    }
    candidates.into_iter().find(|path|path.is_file())
}

pub async fn execute(db: Arc<Database>, cfg:&Settings, job:&Job, session:&Session) -> Result<String> {
    if cfg.workspace_write { bail!("Alma email continuation currently supports tool-free mode only"); }
    let id = match session.agent_session.as_deref() {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => {
            // Fresh email task without an existing thread: create one now.
            let created = create_thread(&job.prompt)?;
            db.set_agent_session(&session.id, &created)?;
            created
        },
    };
    if id.is_empty() || id.len()>128 || !id.bytes().all(|b|b.is_ascii_alphanumeric() || b==b'-' || b==b'_') {
        bail!("invalid Alma session ID");
    }
    let executable=runtime().context("Alma's bundled Bun or a recent Node.js runtime was not found")?;
    let mut cmd=Command::new(executable);
    cmd.arg("-e").arg(include_str!("alma_bridge.js"))
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    for key in ["EMA_MAIL_PASSWORD","EMA_ROUTER_API_KEY","EMA_MAIL_ACCESS_CODE"] {cmd.env_remove(key);}
    let mut child=cmd.spawn().context("could not start the Alma bridge")?;
    let mut stdin=child.stdin.take().context("missing bridge stdin")?;
    stdin.write_all(serde_json::to_string(&serde_json::json!({"threadId":id,"prompt":job.prompt}))?.as_bytes()).await?;
    stdin.shutdown().await?;
    drop(stdin);
    let output=tokio::time::timeout(Duration::from_secs(910),child.wait_with_output()).await
        .context("Alma stopped reporting progress; its original session may still be running")??;
    if !output.status.success() {
        bail!("Alma continuation failed: {}",String::from_utf8_lossy(&output.stderr).chars().take(1000).collect::<String>());
    }
    let response=String::from_utf8(output.stdout).context("invalid Alma response")?;
    if response.trim().is_empty() {bail!("Alma completed without a text response");}
    Ok(crate::email_format::excerpt(&response, crate::email_format::RESULT_LIMIT, &cfg.language))
}
