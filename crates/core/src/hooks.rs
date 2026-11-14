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
fn atomic_write(p: &Path, bytes: &[u8]) -> Result<()> {
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
fn shell_quote(s: &str) -> String { format!("'{}'",s.replace('\'',"'\\''")) }
#[cfg(unix)]
fn owned_file(path: &Path, content: &str) -> Result<()> {
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
    let template=r###"#!/bin/sh
# EMA-MANAGED-HOOK: local spool; no SMTP credentials, no remote network.
ACTIVE=__ACTIVE__
SPOOL=__SPOOL__
case "$1" in codex|claude|gemini|opencode|pi) ;; *) exit 0 ;; esac
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
fn backup_once(path:&Path, old:&[u8])->Result<()> {
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
export const EmaReport = async ({ directory }) => ({
  event: async ({ event }) => {
    if (event.type !== "session.idle") return;
    const props = event.properties ?? event.data ?? {};
    const payload = JSON.stringify({
      type: "session.idle", session_id: props.sessionID ?? props.sessionId ?? "",
      cwd: directory ?? "", response: ""
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
    let js=r###"// EMA-MANAGED-HOOK: global Pi extension.
import { spawn } from "node:child_process";
const script = __SCRIPT__;
export default function (pi) {
  pi.on("agent_end", async (event, ctx) => {
    if (event.willRetry) return;
    const last = (event.messages ?? []).filter(m => m.role === "assistant").at(-1);
    const response = (last?.content ?? []).filter(x => x.type === "text")
      .map(x => x.text).join("\n").slice(0, 2000);
    const payload = JSON.stringify({
      type: "agent_end", session_id: ctx.sessionManager?.getSessionFile?.() ?? "",
      cwd: ctx.cwd ?? "", response
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
            ("pi",install_pi(&home,&shim))
        ] {
            if let Err(err)=result {tracing::warn!(agent=name,"hook sync skipped: {err:#}");}
        }
    }
    #[cfg(not(unix))] {let _=root;}
    Ok(())
}
fn field<'a>(v:&'a Value,keys:&[&str])->&'a str{
    keys.iter().find_map(|k|v.get(*k).and_then(Value::as_str)).unwrap_or("")
}
fn clean(s:&str,limit:usize)->String{
    s.chars().filter(|ch|!ch.is_control()||*ch=='\n'||*ch=='\t')
        .take(limit).collect::<String>().trim().to_string()
}
fn format_report(agent:&str,v:&Value)->Option<(String,String,String)> {
    let typ=field(v,&["type","hook_event_name"]);
    if !matches!((agent,typ),
        ("codex","agent-turn-complete")|("claude","Stop")|("gemini","AfterAgent")
        |("opencode","session.idle")|("pi","agent_end")){return None;}
    let native_session=field(v,&["thread-id","session_id","sessionID"]).to_owned();
    let cwd=field(v,&["cwd"]);
    let project=Path::new(cwd).file_name().and_then(|x|x.to_str()).unwrap_or("unknown");
    let answer=clean(field(v,&["last-assistant-message","last_assistant_message","prompt_response","response"]),2000);
    let subject=format!("EMA Report | {} | Completed",agent.to_ascii_uppercase());
    let body=format!("Agent: {agent}\nStatus: completed\nProject: {}\nSession: {}\n\nResult:\n{}\n\nLocal Agent hook report; this is not a new EMA task.",
        clean(project,100),clean(&native_session,128),
        if answer.is_empty(){"Completed; final response unavailable."}else{&answer});
    Some((native_session,subject,body))
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
        if !["codex","claude","gemini","opencode","pi"].contains(&agent) {
            let _=fs::remove_file(&path);continue;
        }
        let bytes=fs::read(&path)?;
        let v:Value=match serde_json::from_slice(&bytes){
            Ok(v)=>v,Err(_)=>{let _=fs::remove_file(&path);continue;}
        };
        if let Some((session,subject,body))=format_report(agent,&v){
            if session.is_empty()||!db.is_ema_agent_session(agent,&session)? {
                let mut hash=Sha256::new();
                hash.update(agent.as_bytes());hash.update(&bytes);
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
