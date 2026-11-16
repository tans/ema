//! Continue an existing Alma thread using its local thread-sync protocol.
use anyhow::{bail, Context, Result};
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::{io::AsyncWriteExt, process::Command};
use crate::{config::Settings, db::{Job, Session}};

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

pub async fn execute(cfg:&Settings, job:&Job, session:&Session) -> Result<String> {
    if cfg.workspace_write { bail!("Alma email continuation currently supports tool-free mode only"); }
    let id=session.agent_session.as_deref().context("the original Alma session ID is missing")?;
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
