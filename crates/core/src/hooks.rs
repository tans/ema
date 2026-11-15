//! EMA-owned local completion hooks. Agent processes never receive SMTP credentials.
use anyhow::{Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{fs, path::{Path, PathBuf}, time::{SystemTime, UNIX_EPOCH}};
use crate::{config::Settings, db::Database};

const MAX_EVENT: u64 = 65_536;
fn hook_dir(root: &Path) -> PathBuf { root.join("hooks") }
fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() }

fn private_dir(p: &Path) -> Result<()> {
    fs::create_dir_all(p)?;
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(p, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
pub(crate) fn atomic_write(p: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    if let Some(parent)=p.parent() { fs::create_dir_all(parent)?; }
    let tmp=p.with_extension(format!("ema-{}", uuid::Uuid::new_v4()));
    let mut options=fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)] {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f=options.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    fs::rename(tmp,p)?;
    Ok(())
}
#[cfg(unix)]
pub(crate) fn shell_quote(s: &str) -> String { format!("'{}'",s.replace('\'',"'\\''")) }
#[cfg(unix)]
pub(crate) fn owned_file(path: &Path, content: &str) -> Result<()> {
    if path.exists() {
        let existing=fs::read_to_string(path)?;
        if existing==content { return Ok(()); }
        if !existing.contains("EMA-MANAGED-HOOK") {
            tracing::warn!(file=%path.display(),"existing user plugin not overwritten");
            return Ok(());
        }
    }
    atomic_write(path,content.as_bytes())
}
#[cfg(unix)]
fn install_shim(root: &Path) -> Result<PathBuf> {
    let dir=hook_dir(root);
    private_dir(&dir)?;
    private_dir(&dir.join("inbox"))?;
    let shim=dir.join("emit.sh");
    if shim.exists() && !fs::read_to_string(&shim)?.contains("EMA-MANAGED-HOOK") {
        anyhow::bail!("refusing to take over an existing non-EMA hook executable");
    }
    let template=r###"#!/bin/sh
# EMA-MANAGED-HOOK: local spool; no SMTP credentials, no remote network.
ACTIVE=__ACTIVE__
SPOOL=__SPOOL__
case "$1" in codex|claude|gemini|opencode|pi|omp|kimi|zcode|hermes|openclaw|dsh) ;; *) exit 0 ;; esac
[ -r "$ACTIVE" ] || exit 0
LAST="$(cat "$ACTIVE" 2>/dev/null)"
NOW="$(date +%s)"
case "$LAST" in ''|*[!0-9]*) exit 0 ;; esac
[ "$((NOW-LAST))" -ge 0 ] && [ "$((NOW-LAST))" -le 20 ] || exit 0
TMP="$(mktemp "$SPOOL/$1.XXXXXXXX")" || exit 0
chmod 600 "$TMP" 2>/dev/null
if [ "$1" = codex ]; then printf '%s' "$2" | head -c 65536 > "$TMP"
else head -c 65536 > "$TMP"; fi
if [ -s "$TMP" ]; then mv "$TMP" "$TMP.ready" 2>/dev/null || rm -f "$TMP"
else rm -f "$TMP"; fi
exit 0
"###;
    let text=template.replace("__ACTIVE__", &shell_quote(&dir.join("active").to_string_lossy()))
        .replace("__SPOOL__", &shell_quote(&dir.join("inbox").to_string_lossy()));
    owned_file(&shim,&text)?;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&shim,fs::Permissions::from_mode(0o700))?;
    Ok(shim)
}
#[cfg(unix)]
fn install_codex(home: &Path, shim: &Path) -> Result<()> {
    let dir=home.join(".codex");
    if !dir.is_dir() { return Ok(()); }
    let file=dir.join("config.toml");
    let old=if file.exists() {fs::read_to_string(&file)?}else{String::new()};
    let mut doc:toml_edit::DocumentMut=match old.parse() {
        Ok(doc)=>doc, Err(e)=>{tracing::warn!("Codex config parse error, skipping: {e}");return Ok(());}
    };
    let target=shim.to_string_lossy().to_string();
    if let Some(current)=doc.get("notify") {
        let owned=current.as_array().map(|a|a.get(0).and_then(|v|v.as_str())==Some(target.as_str())
            && a.get(1).and_then(|v|v.as_str())==Some("codex")).unwrap_or(false);
        if !owned { tracing::warn!("Codex notify is already assigned; preserving user's notifier"); }
        return Ok(());
    }
    let mut array=toml_edit::Array::new();
    array.push(target);array.push("codex");
    doc["notify"]=toml_edit::Item::Value(toml_edit::Value::Array(array));
    backup_once(&file,old.as_bytes())?;
    atomic_write(&file,doc.to_string().as_bytes())
}
#[cfg(unix)]
pub(crate) fn backup_once(path:&Path, old:&[u8])->Result<()> {
    if !old.is_empty() {
        let backup=path.with_extension("ema-before-hook.bak");
        if !backup.exists(){atomic_write(&backup,old)?;}
    }
    Ok(())
}
#[cfg(unix)]
fn json_hook(file:&Path, event:&str, hook:Value)->Result<()> {
    let old=if file.exists(){fs::read(file)?}else{Vec::new()};
    let mut doc:Value=if old.is_empty(){json!({})}else{
        match serde_json::from_slice(&old) {
            Ok(v)=>v,Err(e)=>{tracing::warn!(file=%file.display(),"preserving invalid JSON: {e}");return Ok(());}
        }
    };
    let Some(obj)=doc.as_object_mut() else {return Ok(());};
    let hooks=obj.entry("hooks").or_insert_with(||json!({}));
    let Some(events)=hooks.as_object_mut() else {return Ok(());};
    let list=events.entry(event).or_insert_with(||json!([]));
    let Some(items)=list.as_array_mut() else {return Ok(());};
    // Avoid a duplicate EMA hook, including a previous installation pointing to the same shim.
    if items.iter().any(|group|group["hooks"].as_array().map(|arr|arr.iter().any(|h|
        h["name"].as_str()==Some("ema-report"))).unwrap_or(false)) {return Ok(());}
    items.push(hook);
    let updated=serde_json::to_vec_pretty(&doc)?;
    if updated!=old {backup_once(file,&old)?;atomic_write(file,&updated)?;}
    Ok(())
}
#[cfg(unix)]
fn install_claude(home:&Path,shim:&Path)->Result<()> {
    let dir=home.join(".claude");
    if !dir.is_dir(){return Ok(());}
    json_hook(&dir.join("settings.json"),"Stop",
        json!({"hooks":[{"name":"ema-report","type":"command",
            "command":format!("{} claude",shell_quote(&shim.to_string_lossy())),"timeout":5}]}))
}
#[cfg(unix)]
fn install_gemini(home:&Path,shim:&Path)->Result<()> {
    let dir=home.join(".gemini");
    if !dir.is_dir(){return Ok(());}
    json_hook(&dir.join("settings.json"),"AfterAgent",
        json!({"hooks":[{"name":"ema-report","type":"command",
            "command":format!("{} gemini",shell_quote(&shim.to_string_lossy())),"timeout":5000}]}))
}
#[cfg(unix)]
fn install_opencode(home:&Path,shim:&Path)->Result<()> {
    let dir=home.join(".config/opencode");
    if !dir.is_dir(){return Ok(());}
    let js=r###"// EMA-MANAGED-HOOK: global OpenCode completion plugin.
import { spawn } from "node:child_process";
const script = __SCRIPT__;
export const EmaReport = async ({ directory, client }) => ({
  event: async ({ event }) => {
    if (event.type !== "session.idle") return;
    const props = event.properties ?? event.data ?? {};
    const id = props.sessionID ?? props.sessionId ?? "";
    let title = "", prompt = "", response = "", contextMessages = [];
    if (id) {
      try {
        const [session, result] = await Promise.all([
          client.session.get({ path: { id } }), client.session.messages({ path: { id } })
        ]);
        title = session.data?.title ?? "";
        const messages = result.data ?? [];
        const text = m => (m?.parts ?? []).filter(p => p.type === "text").map(p => p.text).join("\n");
        const last = messages.findLastIndex(m => m.info?.role === "assistant" && text(m));
        if (last >= 0) {
          response = text(messages[last]);
          const before = messages.slice(0, last);
          const input = before.map(m => m.info?.role === "user").lastIndexOf(true);
          if (input >= 0) {
            prompt = text(before[input]);
            contextMessages = before.slice(0, input).filter(m => ["user", "assistant"].includes(m.info?.role) && text(m))
              .slice(-2).map(m => ({role: m.info.role, text: Array.from(text(m)).slice(0, 1001).join("")}));
          }
        }
      } catch (_) {}
    }
    const payload = JSON.stringify({
      type: "session.idle", session_id: id, session_title: title,
      event_id: event.id ?? event.created ?? Date.now(),
      cwd: directory ?? "", context_messages: contextMessages,
      prompt: prompt.slice(0, 4000), response: response.slice(0, 10000),
      prompt_truncated: prompt.length > 4000, response_truncated: response.length > 10000
    });
    try {
      const p = spawn(script, ["opencode"], { stdio: ["pipe", "ignore", "ignore"] });
      p.on("error", () => {});
      p.stdin.on("error", () => {});
      p.stdin.end(payload);
    } catch (_) {}
  }
});
"###.replace("__SCRIPT__",&serde_json::to_string(&shim.to_string_lossy().to_string())?);
    owned_file(&dir.join("plugins/ema-report.js"),&js)
}
#[cfg(unix)]
fn install_pi(home:&Path,shim:&Path)->Result<()> {
    let dir=home.join(".pi/agent");
    if !dir.is_dir(){return Ok(());}
    // Pi's agent_end fires per low-level run and can precede an auto-retry, compaction
    // retry or queued continuation. agent_settled fires exactly once when the run will not
    // continue automatically, so it is the correct single completion signal for reporting.
    // agent_settled carries no messages, so the final assistant text is read back from the
    // read-only session manager (last assistant message that actually has text).
    let js=r###"// EMA-MANAGED-HOOK: global Pi extension. Reports once per settled run.
import { spawn } from "node:child_process";
const script = __SCRIPT__;
export default function (pi) {
  pi.on("agent_settled", async (event, ctx) => {
    if (event.aborted) return;
    const entries = (ctx.sessionManager?.getBranch?.() ?? ctx.sessionManager?.getEntries?.()) || [];
    let response = "", responseId = "", prompt = "", promptIndex = -1;
    const asText = c => Array.isArray(c) ? c.filter(x => x?.type === "text").map(x => x.text).join("\n")
      : (typeof c === "string" ? c : "");
    for (let i = entries.length - 1; i >= 0; i--) {
      const m = entries[i]?.type === "message" ? entries[i].message : null;
      if (m?.role !== "user") continue;
      const c = m.content;
      prompt = Array.isArray(c) ? c.filter(x => x?.type === "text").map(x => x.text).join("\n")
        : (typeof c === "string" ? c : "");
      if (prompt) { promptIndex = i; break; }
    }
    const contextMessages = entries.slice(0, Math.max(promptIndex, 0))
      .filter(e => e?.type === "message" && ["user", "assistant"].includes(e.message?.role) && asText(e.message.content))
      .slice(-2).map(e => ({role: e.message.role, text: Array.from(asText(e.message.content)).slice(0, 1001).join("")}));
    for (let i = entries.length - 1; i >= 0; i--) {
      const entry = entries[i];
      const m = entry && entry.type === "message" ? entry.message : null;
      if (!m || m.role !== "assistant") continue;
      const c = m.content;
      const text = Array.isArray(c)
        ? c.filter(x => x && x.type === "text").map(x => x.text).join("\n")
        : (typeof c === "string" ? c : "");
      if (text) { response = text; responseId = m.responseId || ""; break; }
    }
    const payload = JSON.stringify({
      type: "agent_end",
      session_id: (ctx.sessionManager && ctx.sessionManager.getSessionFile
        ? ctx.sessionManager.getSessionFile() : null) || "",
      event_id: responseId,
      cwd: ctx.cwd || "",
      session_title: pi.getSessionName?.() || "",
      context_messages: contextMessages,
      prompt: prompt.slice(0, 4000),
      response: response.slice(0, 10000),
      prompt_truncated: prompt.length > 4000, response_truncated: response.length > 10000
    });
    try {
      const p = spawn(script, ["pi"], { stdio: ["pipe", "ignore", "ignore"] });
      p.on("error", () => {});
      p.stdin.on("error", () => {});
      p.stdin.end(payload);
    } catch (_) {}
  });
}
"###.replace("__SCRIPT__",&serde_json::to_string(&shim.to_string_lossy().to_string())?);
    owned_file(&dir.join("extensions/ema-report.ts"),&js)
}

#[cfg(unix)]
fn install_kimi(home:&Path,shim:&Path)->Result<()> {
    // New Kimi Code installs use ~/.kimi-code; older kimi-cli uses ~/.kimi.
    let dir=if home.join(".kimi-code").is_dir() {home.join(".kimi-code")}
        else {home.join(".kimi")};
    if !dir.is_dir(){return Ok(());}
    let file=dir.join("config.toml");
    let old=if file.exists(){fs::read_to_string(&file)?}else{String::new()};
    let mut doc:toml_edit::DocumentMut=match old.parse(){
        Ok(v)=>v,Err(e)=>{tracing::warn!("Kimi config invalid, preserving it: {e}");return Ok(());}
    };
    let command=format!("{} kimi",shell_quote(&shim.to_string_lossy()));
    if doc.get("hooks").is_none(){
        doc["hooks"]=toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
    }
    let Some(hooks)=doc.get_mut("hooks").and_then(toml_edit::Item::as_array_of_tables_mut)
        else {tracing::warn!("Kimi hooks format differs; preserving original config");return Ok(());};
    for event in ["Stop","StopFailure"]{
        if hooks.iter().any(|t|t.get("event").and_then(|v|v.as_str())==Some(event)
            && t.get("command").and_then(|v|v.as_str())==Some(command.as_str())){continue;}
        let mut table=toml_edit::Table::new();
        table["event"]=toml_edit::value(event);
        table["command"]=toml_edit::value(command.as_str());
        table["timeout"]=toml_edit::value(5);
        hooks.push(table);
    }
    let updated=doc.to_string();
    if updated!=old {backup_once(&file,old.as_bytes())?;atomic_write(&file,updated.as_bytes())?;}
    Ok(())
}

#[cfg(unix)]
fn install_omp(home:&Path,shim:&Path)->Result<()> {
    // OMP loads user-scope factories from ~/.omp/agent/hooks/{pre,post}/*.ts.
    let dir=home.join(".omp/agent");
    if !dir.is_dir(){return Ok(());}
    let js=r###"// EMA-MANAGED-HOOK: OMP user-level agent_end observer.
import { spawn } from "node:child_process";
const script = __SCRIPT__;
export default function(pi) {
  pi.on("agent_end", async (event, ctx) => {
    if (event.willContinue || event.awaitingAsyncWork) return;
    const messages = event.messages ?? [];
    const last = messages.filter(m => m.role === "assistant").at(-1);
    const input = messages.filter(m => m.role === "user").at(-1);
    const asText = c => typeof c === "string" ? c : (Array.isArray(c) ? c : []).filter(x => x?.type === "text").map(x => x.text).join("\n");
    const response = asText(last?.content);
    const prompt = asText(input?.content);
    const payload = JSON.stringify({
      type: "agent_end", event_id: Date.now(),
      session_id: ctx.sessionManager?.getSessionFile?.() ?? "",
      cwd: ctx.cwd ?? "", prompt: prompt.slice(0, 4000), response: response.slice(0, 10000),
      prompt_truncated: prompt.length > 4000, response_truncated: response.length > 10000
    });
    try {
      const p = spawn(script, ["omp"], { stdio: ["pipe","ignore","ignore"] });
      p.on("error", () => {});
      p.stdin.on("error", () => {});
      p.stdin.end(payload);
    } catch (_) {}
  });
}
"###.replace("__SCRIPT__",&serde_json::to_string(&shim.to_string_lossy().to_string())?);
    owned_file(&dir.join("hooks/post/ema-report.ts"),&js)
}


#[cfg(unix)]
fn install_zcode(home:&Path,shim:&Path)->Result<()> {
    // ZCode runs user hooks exclusively from ~/.zcode/cli/config.json.
    // Do not enable a previously explicitly-disabled global hook collection:
    // doing so would silently activate unrelated third-party commands.
    let folder=home.join(".zcode/cli");
    if !folder.is_dir(){return Ok(());}
    let file=folder.join("config.json");
    let old=if file.exists(){fs::read(&file)?}else{Vec::new()};
    let mut document:Value=if old.is_empty(){json!({})}else{
        match serde_json::from_slice(&old) {
            Ok(value)=>value,
            Err(err)=>{
                tracing::warn!(path=%file.display(),"ZCode config invalid, preserving it: {err}");
                return Ok(());
            }
        }
    };
    let Some(root)=document.as_object_mut() else {return Ok(());};
    let hooks=root.entry("hooks").or_insert_with(||json!({}));
    let Some(hooks)=hooks.as_object_mut() else {
        tracing::warn!("ZCode hooks config is incompatible; preserving existing settings");
        return Ok(());
    };
    if hooks.get("enabled").and_then(Value::as_bool)==Some(false) {
        tracing::warn!("ZCode hooks explicitly disabled; preserving owner preference");
        return Ok(());
    }
    hooks.insert("enabled".into(),json!(true));
    let events=hooks.entry("events").or_insert_with(||json!({}));
    let Some(events)=events.as_object_mut() else {return Ok(());};
    let stop=events.entry("Stop").or_insert_with(||json!([]));
    let Some(groups)=stop.as_array_mut() else {return Ok(());};
    let executable=shim.to_string_lossy().to_string();
    let already_present=groups.iter().any(|group|{
        group.get("hooks").and_then(Value::as_array)
            .map(|hooks| hooks.iter().any(|entry|
                entry.get("command").and_then(Value::as_str)==Some(executable.as_str())
                && entry.get("args").and_then(Value::as_array)
                    .and_then(|arr|arr.first()).and_then(Value::as_str)==Some("zcode")))
            .unwrap_or(false)
    });
    if !already_present {
        groups.push(json!({
            "hooks":[{
                "type":"process","command":executable,
                "args":["zcode"],"enabled":true,"timeoutMs":5000
            }]
        }));
    }
    let updated=serde_json::to_vec_pretty(&document)?;
    if updated!=old {backup_once(&file,&old)?;atomic_write(&file,&updated)?;}
    Ok(())
}

/// Idempotent. Existing unrelated agent settings are retained, Codex conflicts are skipped.
pub fn install(root:&Path)->Result<()> {
    #[cfg(unix)] {
        let shim=install_shim(root)?;
        let Some(home)=std::env::var_os("HOME").map(PathBuf::from) else {return Ok(());};
        for (name,result) in [
            ("codex",install_codex(&home,&shim)),
            ("claude",install_claude(&home,&shim)),
            ("gemini",install_gemini(&home,&shim)),
            ("opencode",install_opencode(&home,&shim)),
            ("pi",install_pi(&home,&shim)),
            ("kimi",install_kimi(&home,&shim)),
            ("omp",install_omp(&home,&shim)),
            ("zcode",install_zcode(&home,&shim))
        ] {
            if let Err(err)=result {tracing::warn!(agent=name,"hook sync skipped: {err:#}");}
        }
        crate::hooks_extra::install(&home,&shim);
    }
    #[cfg(not(unix))] {let _=root;}
    Ok(())
}
#[path = "hook_report.rs"]
mod report;

fn format_report_in_language(agent: &str, v: &Value, language: &str) -> Option<(String, String, String)> {
    report::format(agent, v, language)
}
#[cfg(test)]
fn format_report(agent: &str, v: &Value) -> Option<(String, String, String)> {
    format_report_in_language(agent, v, "en")
}
fn drain(root:&Path,cfg:&Settings,db:&Database)->Result<()> {
    let dir=hook_dir(root).join("inbox");
    if !dir.is_dir(){return Ok(());}
    let mut entries:Vec<_>=fs::read_dir(dir)?.filter_map(|r|r.ok()).collect();
    entries.sort_by_key(|x|x.file_name());
    for entry in entries.into_iter().take(50){
        let path=entry.path();
        if path.extension().and_then(|s|s.to_str())!=Some("ready"){continue;}
        if !fs::symlink_metadata(&path)?.file_type().is_file(){continue;}
        let len=fs::metadata(&path)?.len();
        if len==0||len>MAX_EVENT{let _=fs::remove_file(&path);continue;}
        let name=entry.file_name().to_string_lossy().to_string();
        let agent=name.split('.').next().unwrap_or("");
        if !["codex","claude","gemini","opencode","pi","omp","kimi","zcode","hermes","openclaw","dsh"].contains(&agent) {
            let _=fs::remove_file(&path);continue;
        }
        let bytes=fs::read(&path)?;
        let v:Value=match serde_json::from_slice(&bytes){
            Ok(v)=>v,Err(_)=>{let _=fs::remove_file(&path);continue;}
        };
        if let Some((session,subject,body))=format_report_in_language(agent,&v,&cfg.language){
            if session.is_empty()||!db.is_ema_agent_session(agent,&session)? {
                let mut hash=Sha256::new();
                hash.update(agent.as_bytes());hash.update(&bytes);
                // Native turn IDs are stable; providers without IDs use a minute
                // window to avoid suppressing identical legitimate future turns.
                if !["turn-id","event_id","timestamp"].iter().any(|k| v.get(*k)
                    .and_then(Value::as_str).map(|x| !x.trim().is_empty()).unwrap_or(false)) {
                    let modified=fs::metadata(&path)?.modified()?
                        .duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                    hash.update((modified / 60).to_le_bytes());
                }
                let id=format!("{:x}",hash.finalize());
                db.queue_hook_report(&id,cfg.mail.address.trim(),&subject,&body)?;
            }
        }
        fs::remove_file(&path)?;
    }
    Ok(())
}
/// Send notifications only while EMA is running and has a configured mailbox.
pub fn tick(root:&Path,cfg:&Settings,db:&Database)->Result<()> {
    let dir=hook_dir(root);
    if cfg.mail.address.trim().is_empty(){
        let _=fs::remove_file(dir.join("active"));
        return Ok(());
    }
    private_dir(&dir)?;
    private_dir(&dir.join("inbox"))?;
    atomic_write(&dir.join("active"),now().to_string().as_bytes())?;
    drain(root,cfg,db).context("hook event processing")
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
        assert_eq!(subject, "EMA Report | PI | Completed");
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
        install_pi(&home, &shim).unwrap();
        let ext = fs::read_to_string(home.join(".pi/agent/extensions/ema-report.ts")).unwrap();
        assert!(ext.contains("EMA-MANAGED-HOOK"));
        assert!(ext.contains("agent_settled"), "must hook the final settled signal");
        assert!(!ext.contains("Date.now()"), "volatile id would defeat dedup");
        assert!(ext.contains(&serde_json::to_string(shim.to_str().unwrap()).unwrap()));
        // Idempotent and non-destructive to other extensions.
        let other = home.join(".pi/agent/extensions/other.ts");
        fs::write(&other, b"// keep me").unwrap();
        install_pi(&home, &shim).unwrap();
        assert_eq!(fs::read_to_string(&other).unwrap(), "// keep me");
    }

    #[test]
    fn pi_spools_and_dedupes_without_smtp() {
        let sandbox = Sandbox::new();
        let home = sandbox.0.join("home");
        let root = sandbox.0.join("root");
        fs::create_dir_all(home.join(".pi/agent")).unwrap();
        let shim = install_shim(&root).unwrap();
        install_pi(&home, &shim).unwrap();
        let (db, cfg) = db_and_cfg(&sandbox);
        tick(&root, &cfg, &db).unwrap();
        let payload = pi_payload();
        // The extension spawns `shim pi` and pipes the payload on stdin.
        for _ in 0..2 {
            let mut child = Command::new(&shim).arg("pi")
                .stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
            child.stdin.take().unwrap().write_all(payload.as_bytes()).unwrap();
            assert!(child.wait().unwrap().success());
        }
        drain(&root, &cfg, &db).unwrap();
        let pending = db.pending_outbox(false).unwrap();
        assert_eq!(pending.len(), 1, "identical settled payloads must dedup to one report");
        assert_eq!(pending[0].subject, "EMA Report | PI | Completed");
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
        assert_eq!(subject, "EMA Report | CODEX | Completed");
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
        install_codex(&home, &shim).unwrap();
        let toml = fs::read_to_string(home.join(".codex/config.toml")).unwrap();
        assert!(toml.contains("model = \"kept\""), "must preserve unrelated config");
        assert!(toml.contains("notify"));
        assert!(toml.contains(shim.to_str().unwrap()));
        // A pre-existing non-EMA notify must not be overwritten.
        let home2 = sandbox.0.join("home2");
        fs::create_dir_all(home2.join(".codex")).unwrap();
        fs::write(home2.join(".codex/config.toml"),
            "notify = [\"/opt/other-notifier\", \"turn-ended\"]\n").unwrap();
        install_codex(&home2, &shim).unwrap();
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
        assert_eq!(pending[0].subject, "EMA Report | CODEX | Completed");
        let s = db.stats().unwrap();
        assert_eq!(s.hook_reports, 1);
        assert_eq!(s.hook_pending, 1);
    }
}
