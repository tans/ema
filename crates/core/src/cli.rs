//! Safe non-interactive CLI adapters for email-launched sessions.
//! Unknown Agent prefixes are still recognized, but never fall back to Codex.
use anyhow::{bail, Context, Result};
use std::{path::{Path,PathBuf}, process::Stdio, sync::Arc, time::Duration};
use tokio::process::Command;
use crate::{config::Settings, db::{Database,Job,Session}};

/// Any installed CLI is usable. There is no registration whitelist: a prefix
/// is routable as long as the matching executable can be found on this machine
/// (or is a configured absolute path). This replaces the old static allow-list.
fn command_name<'a>(agent:&'a str,config:&'a Settings)->Option<&'a str> {
    match agent {
        // Configured absolute/custom paths win for codex; the rest resolve by
        // the agent's own command name.
        "codex"=>Some(&config.codex_command),
        "cc"=>Some("claude"),
        _=>Some(agent)
    }
}

/// Whether an email reply can attempt to resume this Agent: the executable
/// exists locally. Actual continuation may still fail later with a concrete
/// error (missing session file, unavailable cwd, CLI error), which is
/// reported back instead of being refused up front.
pub fn resume_available(agent:&str)->bool {
    match agent {
        "alma"=>true,
        "cc"=>resolve_name("claude").is_some(),
        other=>resolve_name(other).is_some(),
    }
}

/// Look up an executable by its command name across PATH plus the common
/// per-user bins (GUI-launched apps often lack the shell PATH).
pub fn resolve_name(name:&str)->Option<PathBuf> {
    let path=PathBuf::from(name);
    if path.is_absolute() || name.contains(std::path::MAIN_SEPARATOR) {
        return path.is_file().then_some(path);
    }
    let mut dirs:Vec<PathBuf>=std::env::var_os("PATH")
        .map(|value|std::env::split_paths(&value).collect()).unwrap_or_default();
    dirs.extend(["/opt/homebrew/bin","/usr/local/bin","/usr/bin"].into_iter().map(PathBuf::from));
    if let Some(home)=std::env::var_os("HOME").map(PathBuf::from) {
        for sub in [".local/bin",".cargo/bin",".bun/bin",".npm-global/bin",".local/share/mise/shims",
                    ".volta/bin",".asdf/shims",".nix-profile/bin"] {
            dirs.push(home.join(sub));
        }
    }
    dirs.into_iter().map(|dir|dir.join(name)).find(|candidate|candidate.is_file())
}

/// Resolve the executable EMA would launch for this Agent: honour a
/// configured absolute path (codex), then fall back to NAME lookup.
pub fn resolve(agent:&str,config:&Settings)->Option<PathBuf> {
    resolve_name(command_name(agent,config)?)
}

pub async fn execute(db:Arc<Database>,cfg:&Settings,job:&Job,session:&Session)->Result<String> {
    let cwd=session.cwd.as_deref().context("session working directory is missing")?;
    let path=Path::new(cwd).canonicalize()?;
    if !path.is_dir() {bail!("working directory is unavailable");}
    let executable=resolve(&session.agent,cfg)
        .with_context(||format!("{} CLI not found. Install it or configure a supported executable.",session.agent))?;
    let mut cmd=Command::new(&executable);
    if session.agent=="opencode" {
        // Fresh tasks (no native session yet) start a new OpenCode session;
        // the JSON event stream carries the new session ID which we persist
        // for later continuations.
        if let Some(id)=session.agent_session.as_deref() {
            let mut list=Command::new(&executable);
            list.args(["session","list","--format","json","--max-count","10000"])
                .current_dir(&path).stdin(Stdio::null()).kill_on_drop(true);
            for key in ["EMA_MAIL_PASSWORD","EMA_ROUTER_API_KEY","EMA_MAIL_ACCESS_CODE"]{list.env_remove(key);}
            let listed=tokio::time::timeout(Duration::from_secs(15),list.output()).await??;
            if !listed.status.success() {bail!("could not verify the original OpenCode session");}
            let sessions:serde_json::Value=serde_json::from_slice(&listed.stdout).context("invalid OpenCode session list")?;
            if !sessions.as_array().map(|items|items.iter().any(|item|item["id"].as_str()==Some(id))).unwrap_or(false) {
                bail!("the original OpenCode session is unavailable");
            }
            cmd.args(["run","--format","json","--session",id,"--"]);
        } else {
            cmd.args(["run","--format","json","--"]);
        }
        cmd.arg(&job.prompt)
            .current_dir(&path).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        for key in ["EMA_MAIL_PASSWORD","EMA_ROUTER_API_KEY","EMA_MAIL_ACCESS_CODE"]{cmd.env_remove(key);}
        let output=tokio::time::timeout(Duration::from_secs(900),cmd.output()).await
            .context("Agent exceeded 15-minute execution timeout")??;
        if !output.status.success() {bail!("OpenCode failed: {}",failure_detail(&output));}
        let mut answer=String::new();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Ok(event)=serde_json::from_str::<serde_json::Value>(line) {
                if event["type"]=="error" {
                    bail!("OpenCode reported a generation error: {}", crate::email_format::excerpt(&event.to_string(), 1000, &cfg.language));
                }
                // Persist the freshly created session ID from the event stream.
                if let Some(id)=event.get("sessionID").and_then(|v|v.as_str()) {
                    if !id.trim().is_empty() { db.set_agent_session(&session.id, id)?; }
                }
                if event["type"]=="text" {
                    if let Some(text)=event["part"]["text"].as_str() {answer.push_str(text);}
                }
            }
        }
        if answer.trim().is_empty() {bail!("OpenCode completed without a final response");}
        return Ok(crate::email_format::excerpt(&answer, crate::email_format::RESULT_LIMIT, &cfg.language));
    }
    if matches!(session.agent.as_str(),"pi"|"omp") {
        // Fresh tasks have no agent-side session yet: let the CLI create one in
        // an isolated per-EMA-session directory so it can be found and resumed
        // after the run. Existing sessions are resumed by their file path.
        let fresh_dir = if session.agent_session.is_none() {
            let dir = std::env::temp_dir().join("ema-agent-sessions").join(&session.id);
            std::fs::create_dir_all(&dir)?;
            Some(dir)
        } else { None };
        if session.agent=="pi" {
            if let Some(id)=session.agent_session.as_deref() {
                let session_path=Path::new(id).canonicalize().context("the original session file is unavailable")?;
                if !session_path.is_file() {bail!("the original session file is unavailable");}
                cmd.args(["-p","--session"]).arg(session_path)
                    .arg("--");
            } else {
                cmd.arg("-p").arg("--session-dir").arg(fresh_dir.as_ref().unwrap())
                    .arg("--");
            }
        } else {
            if let Some(id)=session.agent_session.as_deref() {
                let session_path=Path::new(id).canonicalize().context("the original session file is unavailable")?;
                if !session_path.is_file() {bail!("the original session file is unavailable");}
                cmd.arg("-p").arg("--resume").arg(session_path)
                    .arg("--");
            } else {
                cmd.arg("-p").arg("--session-dir").arg(fresh_dir.as_ref().unwrap())
                    .arg("--");
            }
        }
        cmd.arg(&job.prompt).current_dir(&path)
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
        for key in ["EMA_MAIL_PASSWORD","EMA_ROUTER_API_KEY","EMA_MAIL_ACCESS_CODE"]{cmd.env_remove(key);}
        let output=tokio::time::timeout(Duration::from_secs(900),cmd.output()).await
            .context("Agent exceeded 15-minute execution timeout")??;
        // Persist the freshly created session file so a later reply can resume it.
        if let Some(dir)=fresh_dir {
            if let Some(latest)=latest_session_file(&dir) {
                db.set_agent_session(&session.id, &latest)?;
            }
        }
        if !output.status.success() {bail!("{} failed: {}",session.agent,failure_detail(&output));}
        let answer=crate::email_format::excerpt(&String::from_utf8_lossy(&output.stdout), crate::email_format::RESULT_LIMIT, &cfg.language);
        if answer.trim().is_empty() {bail!("Agent completed without a final response");}
        return Ok(answer);
    }
    if session.agent=="hermes" {
        // Non-interactive one-shot via the Hermes CLI. The stream-json format
        // ends with a machine-readable result event carrying the session ID
        // (persisted so a later email reply can resume the same conversation).
        cmd.arg("chat").arg("-q").arg(&job.prompt)
            .arg("--oneshot").arg("--format").arg("stream-json")
            .arg("--source").arg("tool");
        if let Some(id)=session.agent_session.as_deref(){cmd.arg("--resume").arg(id);}
        cmd.current_dir(&path).stdin(Stdio::null()).stdout(Stdio::piped())
            .stderr(Stdio::piped()).kill_on_drop(true);
        for key in ["EMA_MAIL_PASSWORD","EMA_ROUTER_API_KEY","EMA_MAIL_ACCESS_CODE"]{cmd.env_remove(key);}
        let output=tokio::time::timeout(Duration::from_secs(3600),cmd.output()).await
            .context("Hermes exceeded 1-hour execution timeout")??;
        let mut result=Option::<serde_json::Value>::None;
        let mut sid=String::new();
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Ok(event)=serde_json::from_str::<serde_json::Value>(line) {
                if let Some(id)=event.get("session_id").and_then(|v|v.as_str()) {
                    if !id.trim().is_empty() {sid=id.to_string();}
                }
                if event["type"]=="result" {result=Some(event);}
            }
        }
        let result=result.context("Hermes did not return a result event")?;
        if !sid.is_empty() {db.set_agent_session(&session.id,&sid)?;}
        let failed=(!output.status.success())
            || result.get("error").map(|v|!v.is_null()).unwrap_or(false)
            || result.get("exit_code").and_then(|v|v.as_i64()).is_some_and(|c|c!=0);
        if failed {
            let reason=result.get("error").filter(|v|!v.is_null()).map(|v|v.to_string())
                .unwrap_or_else(||failure_detail(&output));
            bail!("Hermes failed: {}", crate::email_format::excerpt(&reason, 1000, &cfg.language));
        }
        let answer=crate::email_format::excerpt(
            result.get("text").and_then(|v|v.as_str()).unwrap_or(""),
            crate::email_format::RESULT_LIMIT, &cfg.language);
        if answer.trim().is_empty() {bail!("Hermes completed without a final response");}
        return Ok(answer);
    }
    match session.agent.as_str() {
        "cc"=>{
            cmd.arg("-p").arg(&job.prompt)
               .arg("--output-format").arg("json");
            if let Some(id)=session.agent_session.as_deref(){cmd.arg("--resume").arg(id);}
        }
        "gemini"=>{
            cmd.arg("-p").arg(&job.prompt)
                .arg("--output-format").arg("json");
            if let Some(id)=session.agent_session.as_deref(){cmd.arg("--resume").arg(id);}
        }
        other=>bail!("no execution adapter for {other}: the CLI was found, but EMA has not implemented a non-interactive launch for this Agent"),
    };
    cmd.current_dir(path).stdin(Stdio::null()).stdout(Stdio::piped())
        .stderr(Stdio::piped()).kill_on_drop(true);
    for key in ["EMA_MAIL_PASSWORD","EMA_ROUTER_API_KEY","EMA_MAIL_ACCESS_CODE"]{cmd.env_remove(key);}
    let output=tokio::time::timeout(Duration::from_secs(900),cmd.output()).await
        .context("Agent exceeded 15-minute execution timeout")?
        .context("failed to launch Agent CLI")?;
    if !output.status.success() {
        bail!("{} exited {}: {}",session.agent,output.status,failure_detail(&output));
    }
    if output.stdout.len()>256*1024 {bail!("Agent returned an oversized JSON response");}
    let data:serde_json::Value=serde_json::from_slice(&output.stdout)
        .context("Agent did not return a valid JSON result")?;
    if data.get("is_error").and_then(|v|v.as_bool())==Some(true) || data.get("error")
            .map(|v|!v.is_null()).unwrap_or(false) {
        let reason = data.get("error").filter(|v|!v.is_null())
            .or_else(||data.get("result")).or_else(||data.get("errors")).unwrap_or(&data);
        bail!("Agent reported an unsuccessful result: {}", crate::email_format::excerpt(&reason.to_string(), 1000, &cfg.language));
    }
    let answer=crate::email_format::excerpt(match session.agent.as_str(){
        "cc"=>data.get("result").and_then(|v|v.as_str()),
        "gemini"=>data.get("response").and_then(|v|v.as_str()),
        _=>None
    }.unwrap_or(""), crate::email_format::RESULT_LIMIT, &cfg.language);
    if let Some(id)=data.get("session_id").and_then(|v|v.as_str()){
        if !id.trim().is_empty(){db.set_agent_session(&session.id,id)?;}
    }
    if answer.trim().is_empty() {bail!("Agent completed without a final response");}
    Ok(answer)
}

// Some headless CLIs put a JSON authorization error on stdout and leave stderr empty.
fn failure_detail(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let reason = if stderr.trim().is_empty() { String::from_utf8_lossy(&output.stdout) } else { stderr };
    reason.chars().take(1000).collect()
}

/// Newest `.jsonl` session file inside `dir` (created most recently), if any.
fn latest_session_file(dir: &Path) -> Option<String> {
    let entries = std::fs::read_dir(dir).ok()?;
    entries.filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| (e.path(), e.metadata().and_then(|m| m.modified()).ok()))
        .filter_map(|(p, m)| {
            if p.extension().map(|x| x == "jsonl").unwrap_or(false) { Some((p, m)) } else { None }
        })
        .max_by_key(|(_, m)| m.clone().unwrap_or(std::time::SystemTime::UNIX_EPOCH))
        .map(|(p, _)| p.to_string_lossy().into_owned())
}
