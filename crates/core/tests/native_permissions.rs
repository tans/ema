#![cfg(unix)]
use ema_core::{agent, config::Settings, db::{Database, Job}};
use std::{path::PathBuf, sync::Arc, os::unix::fs::PermissionsExt};

struct Sandbox(PathBuf);
impl Sandbox {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("ema-native-permissions-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

// Run all adapter calls in a separate process so PATH/HOME and credentials
// cannot affect parallel tests or the user's installed Agents/configuration.
#[test]
fn native_permissions_with_isolated_fake_clis() {
    let sandbox = Sandbox::new();
    let bin = sandbox.0.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    for name in ["codex", "claude", "gemini", "opencode", "pi", "omp", "kimi"] {
        let path = bin.join(name);
        std::fs::write(&path, FAKE_CLI).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "capture_native_commands", "--nocapture"])
        .env("EMA_PERMISSION_TEST_DIR", &sandbox.0).env("PATH", &bin).env("HOME", &sandbox.0)
        .env("TMPDIR", &sandbox.0)
        .env("EMA_MAIL_PASSWORD", "fake-mail-secret")
        .env("EMA_ROUTER_API_KEY", "fake-router-secret")
        .env("EMA_MAIL_ACCESS_CODE", "fake-access-secret")
        .env("OPENCODE_PERMISSION", r#"{"edit":"ask"}"#)
        .output().unwrap();
    assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
}

#[tokio::test]
#[ignore = "run only by isolated parent"]
async fn capture_native_commands() {
    let root = PathBuf::from(std::env::var_os("EMA_PERMISSION_TEST_DIR").unwrap());
    let db = Arc::new(Database::open(&root.join("test.db")).unwrap());
    let native = root.join("original.jsonl");
    std::fs::write(&native, "{}\n").unwrap();
    let prompt = "Modify files using the Agent's configured policy. $(never-execute); `literal`";
    for write in [false, true] {
        let mut cfg = Settings::default();
        cfg.workspace_write = write;
        cfg.codex_command = root.join("bin/codex").to_string_lossy().into_owned();
        for name in ["codex", "cc", "gemini", "opencode", "pi", "omp"] {
            for resume in [false, true] {
                let mid = format!("<{}@test>", uuid::Uuid::new_v4());
                db.store_inbound(&mid, "owner@test", None, name, "_agent_memory", "Task", prompt,
                    Some(root.to_str().unwrap()), true, "ema@test", prompt, None, "en").unwrap();
                let mut session = db.find_by_references(&[mid]).unwrap().unwrap();
                if resume {
                    let id = if matches!(name, "pi" | "omp") { native.to_str().unwrap() } else { "native-original" };
                    db.set_agent_session(&session.id, id).unwrap();
                    session.agent_session = Some(id.into());
                }
                let job_id: String = rusqlite::Connection::open(root.join("test.db")).unwrap()
                    .query_row("SELECT id FROM jobs WHERE session_id=?1", [&session.id], |row| row.get(0)).unwrap();
                let job = Job { id: job_id, session_id: session.id.clone(), prompt: prompt.into() };
                let answer = agent::execute(db.clone(), cfg.clone(), job.clone(), session.clone()).await.unwrap();
                assert_eq!(answer, "native answer", "{name}, resume={resume}, legacy={write}");
                let command = if name == "cc" { "claude" } else { name };
                let args = std::fs::read_to_string(root.join(format!("{command}.args"))).unwrap();
                let argv: Vec<_> = args.lines().collect();
                for forbidden in ["--sandbox", "--approval-mode", "--permission-mode", "--tools", "--no-extensions", "--no-mcp", "--no-approve", "--no-lsp", "--pure", "--max-turns", "--yolo", "--full-auto", "--dangerously-skip-permissions", "--dangerously-bypass-approvals-and-sandbox"] {
                    assert!(!argv.iter().any(|arg| *arg == forbidden || arg.starts_with(&format!("{forbidden}="))), "{name} injected {forbidden}: {args}");
                }
                let env = std::fs::read_to_string(root.join(format!("{command}.env"))).unwrap();
                assert!(env.lines().any(|line| line == "OPENCODE_PERMISSION={\"edit\":\"ask\"}"), "native user policy must be inherited");
                for forbidden in ["EMA_MAIL_PASSWORD=", "EMA_ROUTER_API_KEY=", "EMA_MAIL_ACCESS_CODE="] {
                    assert!(!env.lines().any(|line| line.starts_with(forbidden)), "{name} leaked/overrode {forbidden}");
                }
                if name == "codex" {
                    assert_eq!(std::fs::read_to_string(root.join("codex.stdin")).unwrap(), prompt);
                } else { assert!(argv.contains(&prompt), "prompt must be one literal argument"); }
                if resume {
                    let flag = if name == "codex" { "resume" } else if matches!(name, "pi" | "opencode") { "--session" } else { "--resume" };
                    let at = argv.iter().position(|arg| *arg == flag).unwrap();
                    assert_eq!(argv[at + 1], session.agent_session.as_deref().unwrap());
                }
                assert!(db.session(&session.id).unwrap().agent_session.is_some(), "must retain native session for {name}");
                // A headless permission failure must be returned, never approved/retried.
                std::fs::write(root.join("fail"), "").unwrap();
                let error = agent::execute(db.clone(), cfg.clone(), job.clone(), session.clone()).await.unwrap_err().to_string();
                std::fs::remove_file(root.join("fail")).unwrap();
                assert!(error.contains("native approval required"), "{name}: {error}");
                if matches!(name, "codex" | "cc" | "gemini" | "opencode") {
                    for status in [0, 23] {
                        std::fs::write(root.join("structured-fail"), format!("{status}\n")).unwrap();
                        let result = agent::execute(db.clone(), cfg.clone(), job.clone(), session.clone()).await;
                        std::fs::remove_file(root.join("structured-fail")).unwrap();
                        assert!(result.is_err(), "{name} must not report structured failures as success");
                        assert!(result.unwrap_err().to_string().contains("native approval required"), "{name} must retain the native failure reason for exit {status}");
                    }
                }
            }
        }
    }
}

#[test]
fn legacy_setting_round_trips_without_authorization_gate() {
    let sandbox = Sandbox::new();
    for write in [false, true] {
        let mut cfg = Settings::default();
        cfg.workspace_write = write;
        cfg.enabled = true;
        cfg.mail.address = "ema@test".into(); cfg.mail.username = "ema@test".into();
        cfg.mail.imap_host = "imap.test".into(); cfg.mail.smtp_host = "smtp.test".into();
        cfg.allowlist = vec!["owner@test".into()];
        cfg.write(&sandbox.0).unwrap();
        let restored = Settings::read(&sandbox.0).unwrap();
        assert_eq!(restored.workspace_write, write);
        assert_eq!(restored.mail.address, cfg.mail.address);
        assert_eq!(restored.allowlist, cfg.allowlist);
    }
}


const FAKE_CLI: &str = r##"#!/bin/sh
name=${0##*/}
if [ "$1" = session ]; then
  printf '[{"id":"native-original"}]'
  exit 0
fi
printf '%s\n' "$@" > "$name.args"
/usr/bin/env > "$name.env"
if [ -f fail ]; then
  printf 'native approval required; cannot confirm in headless mode' >&2
  exit 23
fi
if [ -f structured-fail ]; then
  case "$name" in
    codex) printf '%s\n' '{"type":"turn.failed","error":{"message":"native approval required"}}' ;;
    claude) printf '{"is_error":true,"result":"native approval required"}' ;;
    gemini) printf '{"error":{"message":"native approval required","code":23}}' ;;
    opencode) printf '{"type":"error","error":{"data":{"message":"native approval required"}}}\n' ;;
  esac
  read -r status < structured-fail
  exit "$status"
fi
previous=''
for arg in "$@"; do
  if [ "$previous" = '--session-dir' ]; then printf '{}\n' > "$arg/native.jsonl"; fi
  previous=$arg
done
case "$name" in
  codex)
    /bin/cat > codex.stdin
    printf '%s\n' '{"type":"thread.started","thread_id":"native-created"}' '{"type":"item.completed","item":{"type":"agent_message","text":"native answer"}}'
    ;;
  claude) printf '{"result":"native answer","session_id":"native-created"}' ;;
  gemini) printf '{"response":"native answer","session_id":"native-created"}' ;;
  opencode) printf '{"type":"text","sessionID":"native-created","part":{"text":"native answer"}}\n' ;;
  *) printf 'native answer' ;;
esac
"##;
