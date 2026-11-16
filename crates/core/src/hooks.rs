//! EMA-owned local completion hooks. Agent processes never receive SMTP credentials.
use anyhow::{Context, Result};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{fs, path::{Path, PathBuf}, time::{SystemTime, UNIX_EPOCH}};
use crate::{config::Settings, db::Database};

#[cfg(test)]
use serde_json::json;
#[cfg(test)]
use agent_hooks::adapter::AgentAdapter;

const MAX_EVENT: u64 = 65_536;

fn hook_dir(root: &Path) -> PathBuf { root.join("hooks") }
fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() }

fn private_dir(p: &Path) -> Result<()> {
    fs::create_dir_all(p)?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(p, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn stage(p: &Path, bytes: &[u8]) -> Result<PathBuf> {
    use std::io::Write;
    if let Some(parent) = p.parent() { fs::create_dir_all(parent)?; }
    let tmp = p.with_extension(format!("ema-{}", uuid::Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(tmp)
}

pub(crate) fn atomic_write(p: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = stage(p, bytes)?;
    fs::rename(tmp, p)?;
    Ok(())
}

/// Install (or refresh) the managed spool shim, then reconcile every known
/// agent's hook registration. The registry logs per-agent failures and never
/// aborts the rest of the loop.
pub fn install(root: &Path) -> Result<()> {
    #[cfg(unix)] {
        let shim = install_shim(root)?;
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            agent_hooks::registry::sync(&home, &shim);
        }
    }
    #[cfg(not(unix))] { let _ = root; }
    Ok(())
}

/// The shared spool shim every hook funnels into. Refuses to take over a
/// pre-existing non-EMA executable.
pub(crate) fn install_shim(root: &Path) -> Result<PathBuf> {
    agent_hooks::shim::install(root, &agent_hooks::registry::agent_names())
}

#[path = "hook_report.rs"]
mod report;

#[cfg(test)]
fn format_report_in_language(agent: &str, v: &Value, language: &str) -> Option<(String, String, String)> {
    // All wire payloads run through the agent adapter registry, which reduces the
    // native shape to the canonical event before the report renderer sees it.
    let ev = agent_hooks::registry::find(agent)?.normalize(v)?;
    report::format(agent, &ev, language)
}

#[cfg(test)]
fn format_report(agent: &str, v: &Value) -> Option<(String, String, String)> {
    // Tests assert English output; use the language-aware path so the setting never leaks in.
    format_report_in_language(agent, v, "en")
}

fn drain(root: &Path, cfg: &Settings, db: &Database) -> Result<()> {
    let dir = hook_dir(root).join("inbox");
    if !dir.is_dir() { return Ok(()); }
    let mut entries: Vec<_> = fs::read_dir(dir)?.filter_map(|r| r.ok()).collect();
    entries.sort_by_key(|x| x.file_name());
    for entry in entries.into_iter().take(50) {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("ready") { continue; }
        if !fs::symlink_metadata(&path)?.file_type().is_file() { continue; }
        let len = fs::metadata(&path)?.len();
        if len == 0 || len > MAX_EVENT { let _ = fs::remove_file(&path); continue; }
        let name = entry.file_name().to_string_lossy().to_string();
        let agent = name.split('.').next().unwrap_or("");
        // The registry is the single allow-list: unknown agents are dropped.
        let Some(adapter) = agent_hooks::registry::find(agent) else {
            let _ = fs::remove_file(&path); continue;
        };
        let bytes = fs::read(&path)?;
        let v: Value = match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(err) => {
                // A payload that is exactly the cap almost always means the shim truncated
                // it mid-JSON; surface that instead of dropping it silently.
                if len == MAX_EVENT {
                    tracing::warn!(agent=%agent, "hook payload hit the 64KiB cap and was truncated, dropped: {err}");
                }
                let _ = fs::remove_file(&path); continue;
            }
        };
        // Reduce through the adapter registry; unrecognized wire types are dropped.
        let Some(ev) = adapter.normalize(&v) else {
            let _ = fs::remove_file(&path); continue;
        };
        let Some((session, subject, body)) = report::format(agent, &ev, &cfg.language) else {
            let _ = fs::remove_file(&path); continue;
        };
        if session.is_empty() || !db.is_ema_agent_session(agent, &session)? {
            let mut hash = Sha256::new();
            hash.update(agent.as_bytes());
            // Alma's stable thread/message identity survives usage/serialization changes.
            if agent == "alma" {
                hash.update(ev.session_id.as_bytes());
                hash.update([0]);
                hash.update(ev.event_id.as_bytes());
            } else {
                hash.update(&bytes);
            }
            // Native turn IDs are stable; providers without IDs use a minute
            // window to avoid suppressing identical legitimate future turns.
            let has_native_id = !ev.event_id.trim().is_empty()
                || v.get("timestamp").and_then(Value::as_str)
                    .map(|x| !x.trim().is_empty()).unwrap_or(false);
            if agent != "alma" && !has_native_id {
                let modified = fs::metadata(&path)?.modified()?
                    .duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                hash.update((modified / 60).to_le_bytes());
            }
            let id = format!("{:x}", hash.finalize());
            db.queue_hook_report(&id, cfg.mail.address.trim(), &subject, &body, agent, &session, &ev.cwd)?;
        }
        fs::remove_file(&path)?;
    }
    Ok(())
}

/// Send notifications only while EMA is running and has a configured mailbox.
pub fn tick(root: &Path, cfg: &Settings, db: &Database) -> Result<()> {
    let dir = hook_dir(root);
    if cfg.mail.address.trim().is_empty() {
        let _ = fs::remove_file(dir.join("active"));
        return Ok(());
    }
    private_dir(&dir)?;
    private_dir(&dir.join("inbox"))?;
    atomic_write(&dir.join("active"), now().to_string().as_bytes())?;
    drain(root, cfg, db).context("hook event processing")
}

#[cfg(all(test, unix))]
mod alma_tests {
    use super::*;
    use std::{io::Write, process::{Command, Stdio}};

    struct Sandbox(PathBuf);
    impl Sandbox {
        fn new() -> Self {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/hook-tests").join(uuid::Uuid::new_v4().to_string());
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }
    fn payload() -> Value {
        json!({"hook":"chat.message.didReceive","matcher":".*","input":{
            "threadId":"alma-thread", "message":{"id":"reply-1","role":"assistant",
                "parts":[{"type":"text","text":"fallback"}]},
            "response":{"content":"Alma final reply", "usage":{"outputTokens":12}}
        }})
    }
    #[test]
    fn alma_normalizes_main_reply_only() {
        let v = payload();
        let (session, subject, body) = format_report("alma", &v).expect("Alma main reply");
        assert_eq!(session, "alma-thread");
        assert_eq!(subject, "[ALMA][Round ended]");
        assert!(body.contains("Alma final reply"));
        assert!(!body.contains("Project:"), "empty cwd hides the Project field");
        assert!(!body.contains("outputTokens"));
        let mut parts = v.clone();
        parts["input"]["response"]["content"] = json!([
            {"type":"reasoning","text":"private thought"},
            {"type":"text","text":"visible reply"}]);
        let body = format_report("alma", &parts).unwrap().2;
        assert!(body.contains("visible reply"));
        assert!(!body.contains("private thought"));
        for event in ["chat.message.willSend", "subagent.didComplete"] {
            let mut other = v.clone(); other["hook"] = json!(event);
            assert!(format_report("alma", &other).is_none());
        }
        let mut fallback = v.clone();
        fallback["input"]["response"] = Value::Null;
        assert!(format_report("alma", &fallback).unwrap().2.contains("fallback"));
        fallback["input"]["message"]["role"] = json!("user");
        assert!(!format_report("alma", &fallback).unwrap().2.contains("fallback"));
        let mut long = v.clone();
        long["input"]["response"]["content"] = json!("好".repeat(13_000));
        assert_eq!(format_report("alma", &long).unwrap().2.matches('好').count(), 12_000);
        let mut missing = v;
        missing["input"]["threadId"] = json!("");
        assert!(format_report("alma", &missing).is_none());
    }
    #[test]
    fn alma_installs_and_spools_without_newline_or_smtp() {
        let sandbox = Sandbox::new();
        let home = sandbox.0.join("home");
        let root = sandbox.0.join("queue with ' quote");
        fs::create_dir_all(home.join(".config/alma")).unwrap();
        let config = home.join(".config/alma/hooks.json");
        let original = json!({"extra":true,"hooks":{"other.event":[],
            "chat.message.didReceive":[{"matcher":"keep","hooks":[{
                "type":"command","command":"echo keep","enabled":false}]}]}});
        fs::write(&config, serde_json::to_vec(&original).unwrap()).unwrap();
        let shim = install_shim(&root).unwrap();
        agent_hooks::registry::sync(&home, &shim);
        let installed: Value = serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
        let groups = installed["hooks"]["chat.message.didReceive"].as_array().unwrap();
        assert_eq!(groups.len(), 2, "append an Alma adapter");
        assert_eq!(groups[0], original["hooks"]["chat.message.didReceive"][0]);
        let command = groups[1]["hooks"][0]["command"].as_str().unwrap();
        assert_eq!(groups[1]["hooks"][0]["timeout"], 3000);
        agent_hooks::registry::sync(&home, &shim);
        assert_eq!(fs::read(&config).unwrap(), serde_json::to_vec_pretty(&installed).unwrap());
        let db = Database::open(&root.join("test.db")).unwrap();
        let mut cfg = Settings::default(); cfg.mail.address = "isolated@example.invalid".into();
        tick(&root, &cfg, &db).unwrap();
        for attempt in 0..2 {
            let mut event = payload();
            event["input"]["response"]["usage"]["outputTokens"] = json!(12 + attempt);
            let bytes = serde_json::to_vec(&event).unwrap();
            let mut child = Command::new("sh").args(["-c", command])
                .current_dir("/").env("ALMA_HOOK", "true")
                .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
            child.stdin.take().unwrap().write_all(&bytes).unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(output.status.success()); assert!(output.stdout.is_empty());
            let count = fs::read_dir(root.join("hooks/inbox")).unwrap().count();
            assert_eq!(count, 1, "shim must accept Alma payload without a newline");
            drain(&root, &cfg, &db).unwrap();
        }
        let pending = db.pending_outbox(false).unwrap();
        assert_eq!(pending.len(), 1, "native reply IDs deduplicate");
        assert!(pending[0].body.contains("Alma final reply"));
        assert_eq!(db.stats().unwrap().hook_sent, 0);
    }
    #[test]
    fn alma_lifecycle_preserves_disabled_and_unrelated_rules() {
        let sandbox = Sandbox::new();
        let home = sandbox.0.join("home");
        let root = sandbox.0.join("root");
        fs::create_dir_all(home.join(".config/alma")).unwrap();
        crate::hooks_alma::install(&root, &home).unwrap();
        assert_eq!(crate::hooks_alma::status(&root, &home).unwrap()["enabled"], true);
        let file = home.join(".config/alma/hooks.json");
        let mut config: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        config["hooks"]["chat.message.didReceive"][0]["hooks"][0]["enabled"] = json!(false);
        config["hooks"]["unrelated"] = json!([{"command":"keep"}]);
        fs::write(&file, serde_json::to_vec_pretty(&config).unwrap()).unwrap();
        crate::hooks_alma::install(&root, &home).unwrap();
        assert_eq!(crate::hooks_alma::status(&root, &home).unwrap()["enabled"], false);
        crate::hooks_alma::uninstall(&root, &home).unwrap();
        assert_eq!(crate::hooks_alma::status(&root, &home).unwrap()["installed"], false);
        let config: Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
        assert_eq!(config["hooks"]["unrelated"], json!([{"command":"keep"}]));
        // Uninstall persists across automatic reconciliation until an explicit install.
        let shim = install_shim(&root).unwrap();
        agent_hooks::registry::sync(&home, &shim);
        assert_eq!(crate::hooks_alma::status(&root, &home).unwrap()["installed"], false);
        crate::hooks_alma::install(&root, &home).unwrap();
        assert_eq!(crate::hooks_alma::status(&root, &home).unwrap()["enabled"], true);
        fs::write(&file, b"{invalid").unwrap();
        assert!(crate::hooks_alma::install(&root, &home).is_err());
        assert_eq!(fs::read(&file).unwrap(), b"{invalid");
    }

}

#[cfg(all(test, unix))]
mod pi_codex_tests {
    use super::*;
    use std::{io::Write, process::{Command, Stdio}};

    struct Sandbox(PathBuf);
    impl Sandbox {
        fn new() -> Self {
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/hook-tests").join(uuid::Uuid::new_v4().to_string());
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Sandbox {
        fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); }
    }
    fn db_and_cfg(sandbox: &Sandbox) -> (Database, Settings) {
        let db = Database::open(&sandbox.0.join("root/ema.db")).unwrap();
        let mut cfg = Settings::default();
        cfg.mail.address = "isolated@example.invalid".into();
        cfg.language = "en".into();
        (db, cfg)
    }

    fn pi_payload() -> String {
        // Mirrors the payload EMA's Pi extension emits on agent_settled.
        let v = json!({
            "type": "agent_end",
            "session_id": "/tmp/pi/sessions/demo.jsonl",
            "event_id": "resp-abc-1",
            "cwd": "/tmp/pi/project",
            "response": "Pi finished the task."
        });
        serde_json::to_string(&v).unwrap()
    }

    #[test]
    fn pi_reports_completion_once() {
        let v: Value = serde_json::from_str(&pi_payload()).unwrap();
        let (session, subject, body) = format_report("pi", &v).expect("pi agent_end report");
        assert_eq!(session, "/tmp/pi/sessions/demo.jsonl");
        assert_eq!(subject, "[project][PI][Round ended]");
        assert!(body.contains("Project: project"));
        assert!(body.contains("Pi finished the task."));
        // A wrong event type must be discarded, like every other adapter.
        let mut other = v.clone();
        other["type"] = json!("agent_start");
        assert!(format_report("pi", &other).is_none());
    }

    #[test]
    fn pi_installs_settled_extension() {
        let sandbox = Sandbox::new();
        let home = sandbox.0.join("home");
        let root = sandbox.0.join("root");
        fs::create_dir_all(home.join(".pi/agent")).unwrap();
        let shim = install_shim(&root).unwrap();
        agent_hooks::adapters::Pi.install(&home, &shim).unwrap();
        let ext = fs::read_to_string(home.join(".pi/agent/extensions/ema-report.ts")).unwrap();
        assert!(ext.contains("EMA-MANAGED-HOOK"));
        assert!(ext.contains("agent_settled"), "must hook the final settled signal");
        assert!(!ext.contains("Date.now()"), "volatile id would defeat dedup");
        assert!(ext.contains(&serde_json::to_string(shim.to_str().unwrap()).unwrap()));
        // Idempotent and non-destructive to other extensions.
        let other = home.join(".pi/agent/extensions/other.ts");
        fs::write(&other, b"// keep me").unwrap();
        agent_hooks::adapters::Pi.install(&home, &shim).unwrap();
        assert_eq!(fs::read_to_string(&other).unwrap(), "// keep me");
    }

    #[test]
    fn pi_spools_and_dedupes_without_smtp() {
        let sandbox = Sandbox::new();
        let home = sandbox.0.join("home");
        let root = sandbox.0.join("root");
        fs::create_dir_all(home.join(".pi/agent")).unwrap();
        let shim = install_shim(&root).unwrap();
        agent_hooks::adapters::Pi.install(&home, &shim).unwrap();
        let (db, cfg) = db_and_cfg(&sandbox);
        tick(&root, &cfg, &db).unwrap();
        let payload = pi_payload();
        // The extension spawns `shim pi` and pipes the payload on stdin.
        for attempt in 0..2 {
            let mut child = Command::new(&shim).arg("pi")
                .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
            child.stdin.take().unwrap().write_all(payload.as_bytes()).unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success());
            let _ = attempt;
        }
        drain(&root, &cfg, &db).unwrap();
        let pending = db.pending_outbox(false).unwrap();
        assert_eq!(pending.len(), 1, "identical settled payloads must dedup to one report");
        assert_eq!(pending[0].subject, "[project][PI][Round ended]");
        assert!(pending[0].body.contains("Pi finished the task."));
        let s = db.stats().unwrap();
        assert_eq!(s.hook_reports, 1);
        assert_eq!(s.hook_pending, 1);
        assert_eq!(s.hook_sent, 0);
        // Spool must be empty after a successful drain.
        let remaining = fs::read_dir(root.join("hooks/inbox")).unwrap()
            .filter_map(|e| e.ok()).count();
        assert_eq!(remaining, 0);
    }

    fn codex_payload() -> String {
        // Exact wire shape emitted by codex-rs legacy_notify (final argv argument).
        let v = json!({
            "type": "agent-turn-complete",
            "thread-id": "b5f6c1c2-1111-2222-3333-444455556666",
            "turn-id": "12345",
            "cwd": "/tmp/codex/project",
            "client": "codex-tui",
            "input-messages": ["Rename foo to bar"],
            "last-assistant-message": "Rename complete and verified `cargo build` succeeds."
        });
        serde_json::to_string(&v).unwrap()
    }

    #[test]
    fn codex_reports_from_argv_not_stdin() {
        let v: Value = serde_json::from_str(&codex_payload()).unwrap();
        let (session, subject, body) = format_report("codex", &v).expect("codex report");
        assert_eq!(session, "b5f6c1c2-1111-2222-3333-444455556666");
        assert_eq!(subject, "[project][CODEX][Round ended]");
        assert!(body.contains("Project: project"));
        assert!(body.contains("Rename complete and verified"));
        assert!(!body.contains("Rename foo to bar"), "input messages must not leak into the report");
    }

    #[test]
    fn codex_installs_notify_and_dedupes() {
        let sandbox = Sandbox::new();
        let home = sandbox.0.join("home");
        let root = sandbox.0.join("root");
        fs::create_dir_all(home.join(".codex")).unwrap();
        // Preserve an unrelated existing config value.
        fs::write(home.join(".codex/config.toml"), "model = \"kept\"\n").unwrap();
        let shim = install_shim(&root).unwrap();
        agent_hooks::adapters::Codex.install(&home, &shim).unwrap();
        let toml = fs::read_to_string(home.join(".codex/config.toml")).unwrap();
        assert!(toml.contains("model = \"kept\""), "must preserve unrelated config");
        assert!(toml.contains("notify"));
        assert!(toml.contains(shim.to_str().unwrap()));
        // A pre-existing non-EMA notify must not be overwritten.
        let home2 = sandbox.0.join("home2");
        fs::create_dir_all(home2.join(".codex")).unwrap();
        fs::write(home2.join(".codex/config.toml"),
            "notify = [\"/opt/other-notifier\", \"turn-ended\"]\n").unwrap();
        agent_hooks::adapters::Codex.install(&home2, &shim).unwrap();
        let toml2 = fs::read_to_string(home2.join(".codex/config.toml")).unwrap();
        assert!(toml2.contains("/opt/other-notifier"), "must keep the user's existing notifier");

        let (db, cfg) = db_and_cfg(&sandbox);
        tick(&root, &cfg, &db).unwrap();
        // codex appends the payload as the final argv argument; stdin is unused.
        let payload = codex_payload();
        for _ in 0..2 {
            let out = Command::new(&shim).args(["codex", &payload])
                .stdin(Stdio::null()).stdout(Stdio::piped())
                .output().unwrap();
            assert!(out.status.success());
        }
        drain(&root, &cfg, &db).unwrap();
        let pending = db.pending_outbox(false).unwrap();
        assert_eq!(pending.len(), 1, "same turn-id and payload must dedup to one report");
        assert_eq!(pending[0].subject, "[project][CODEX][Round ended]");
        let s = db.stats().unwrap();
        assert_eq!(s.hook_reports, 1);
        assert_eq!(s.hook_pending, 1);
    }
}
