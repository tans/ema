//! Additional protocol-specific Hook adapters.
//! Config modifications are idempotent and preserve explicit owner denies.
use anyhow::Result;
use serde_json::{json, Value as Json};
use std::{fs, path::{Path, PathBuf}};
use crate::hooks::{atomic_write, backup_once, owned_file, shell_quote};

fn yaml_key(value:&str)->serde_yaml::Value { serde_yaml::Value::String(value.into()) }

fn install_hermes(home:&Path,shim:&Path)->Result<()> {
    let root=home.join(".hermes");
    if !root.is_dir(){return Ok(());}
    let plugin=root.join("plugins/ema-report");
    let manifest=plugin.join("plugin.yaml");
    if manifest.exists() && !fs::read_to_string(&manifest)?.contains("EMA-MANAGED-HOOK"){
        tracing::warn!("Hermes ema-report plugin belongs to another owner; skipping");
        return Ok(());
    }
    let code=r###"# EMA-MANAGED-HOOK: lightweight Hermes lifecycle observer.
import json
import subprocess

SCRIPT = __SCRIPT__

def register(ctx):
    def on_finish(session_id=None, task_id=None, turn_id=None, completed=False,
                  failed=False, interrupted=False, **kwargs):
        event = {
            "hook_event_name": "on_session_end",
            "session_id": session_id or "",
            "turn_id": turn_id or "",
            "event_id": str(task_id or turn_id or ""),
            "completed": bool(completed),
            "failed": bool(failed),
            "interrupted": bool(interrupted),
        }
        try:
            child = subprocess.Popen([SCRIPT, "hermes"], stdin=subprocess.PIPE,
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            child.communicate(json.dumps(event).encode("utf-8"), timeout=2)
        except Exception:
            pass
    ctx.register_hook("on_session_end", on_finish)
"###.replace("__SCRIPT__",&serde_json::to_string(&shim.to_string_lossy().to_string())?);
    let description="# EMA-MANAGED-HOOK\nname: ema-report\nversion: \"1.0\"\ndescription: EMA local turn status emails\nprovides_hooks:\n  - on_session_end\n";
    // Only install in user-owned Hermes, not in project-local directories.
    owned_file(&manifest,description)?;
    owned_file(&plugin.join("__init__.py"),&code)?;
    let conf=root.join("config.yaml");
    let before=if conf.exists(){fs::read_to_string(&conf)?}else{String::new()};
    let mut doc:serde_yaml::Value=if before.trim().is_empty(){
        serde_yaml::Value::Mapping(serde_yaml::Mapping::new())
    }else{
        match serde_yaml::from_str(&before) {
            Ok(v)=>v,Err(err)=>{
                tracing::warn!("Hermes config invalid; do not enable plugin: {err}");
                return Ok(());
            }
        }
    };
    let Some(root_map)=doc.as_mapping_mut() else{return Ok(());};
    if !root_map.contains_key(&yaml_key("plugins")){
        root_map.insert(yaml_key("plugins"),serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
    }
    let Some(plugins)=root_map.get_mut(&yaml_key("plugins")).and_then(serde_yaml::Value::as_mapping_mut)
        else{return Ok(());};
    let off=plugins.get(&yaml_key("disabled")).and_then(serde_yaml::Value::as_sequence)
        .map(|arr|arr.iter().any(|v|v.as_str()==Some("ema-report"))).unwrap_or(false);
    if off{
        tracing::warn!("Hermes plugin explicitly disabled; respecting user's choice");
        return Ok(());
    }
    if !plugins.contains_key(&yaml_key("enabled")){
        plugins.insert(yaml_key("enabled"),serde_yaml::Value::Sequence(Vec::new()));
    }
    let Some(enabled)=plugins.get_mut(&yaml_key("enabled")).and_then(serde_yaml::Value::as_sequence_mut)
        else{return Ok(());};
    if enabled.iter().any(|v|v.as_str()==Some("ema-report")){return Ok(());}
    enabled.push(yaml_key("ema-report"));
    let new=serde_yaml::to_string(&doc)?;
    // YAML is semantic-preserving but may normalize formatting; retain source backup.
    backup_once(&conf,before.as_bytes())?;
    atomic_write(&conf,new.as_bytes())
}

fn install_openclaw(home:&Path,shim:&Path)->Result<()> {
    let root=home.join(".openclaw");
    if !root.is_dir(){return Ok(());}
    let plugin=root.join("extensions/ema-report");
    let manifest=plugin.join("openclaw.plugin.json");
    if manifest.exists() && !fs::read_to_string(&manifest)?.contains("EMA-MANAGED-HOOK"){
        tracing::warn!("OpenClaw ema-report plugin belongs to another owner; skipping");
        return Ok(());
    }
    let script=serde_json::to_string(&shim.to_string_lossy().to_string())?;
    let module=r###"// EMA-MANAGED-HOOK: observe agent completion and pass a short final answer.
import { spawn } from "node:child_process";
const script = __SCRIPT__;
export default {
  id: "ema-report",
  name: "EMA Email Completion Reports",
  register(api) {
    api.on("agent_end", async (event, ctx) => {
      const messages = Array.isArray(event?.messages) ? event.messages : [];
      const last = messages.filter(m => m?.role === "assistant").at(-1);
      let result = "";
      const content = last?.content;
      if (typeof content === "string") result = content;
      else if (Array.isArray(content)) {
        result = content.filter(p => p?.type === "text").map(p => p.text).join("\n");
      }
      const payload = JSON.stringify({
        type: "agent_end",
        session_id: ctx?.sessionKey ?? ctx?.sessionId ?? "",
        event_id: ctx?.runId ?? event?.runId ?? Date.now(),
        completed: !!event?.success,
        failed: event?.success === false,
        response: result.slice(0, 2000)
      });
      try {
        const p = spawn(script, ["openclaw"], { stdio: ["pipe", "ignore", "ignore"] });
        p.on("error", () => {});
        p.stdin.on("error", () => {});
        p.stdin.end(payload);
      } catch (_) {}
    });
  }
};
"###.replace("__SCRIPT__",&script);
    let pkg=json!({"name":"ema-report","version":"1.0.0","type":"module",
        "description":"EMA-MANAGED-HOOK: local agent completion observer",
        "openclaw":{"extensions":["./index.ts"]}});
    let metadata=json!({"id":"ema-report","name":"EMA Email Completion Reports",
        "description":"EMA-MANAGED-HOOK, short completion reports to owner's mailbox",
        "activation":{"onStartup":true},
        "configSchema":{"type":"object","additionalProperties":false}});
    owned_file(&manifest,&serde_json::to_string_pretty(&metadata)?)?;
    owned_file(&plugin.join("package.json"),&serde_json::to_string_pretty(&pkg)?)?;
    owned_file(&plugin.join("index.ts"),&module)?;
    let config=root.join("openclaw.json");
    let before=if config.exists(){fs::read_to_string(&config)?}else{String::new()};
    let mut doc:Json=if before.trim().is_empty(){json!({})} else {
        match json5::from_str(&before){
            Ok(v)=>v,Err(err)=>{
                tracing::warn!("OpenClaw config incompatible; plugin installed but not enabled: {err}");
                return Ok(());
            }
        }
    };
    let Some(root)=doc.as_object_mut() else{return Ok(());};
    let plugins=root.entry("plugins").or_insert_with(||json!({}));
    let Some(plugins)=plugins.as_object_mut() else{return Ok(());};
    if plugins.get("enabled").and_then(Json::as_bool)==Some(false){return Ok(());}
    if plugins.get("deny").and_then(Json::as_array).map(|a|a.iter().any(|v|v.as_str()==Some("ema-report"))).unwrap_or(false){return Ok(());}
    if let Some(allow)=plugins.get("allow").and_then(Json::as_array) {
        if !allow.iter().any(|v|v.as_str()==Some("ema-report")) {
            // The owner has explicitly restricted plugin loading.
            tracing::warn!("OpenClaw plugin allow-list excludes EMA; not overriding it");
            return Ok(());
        }
    }
    let entries=plugins.entry("entries").or_insert_with(||json!({}));
    let Some(entries)=entries.as_object_mut() else{return Ok(());};
    if entries.get("ema-report").and_then(|v|v.get("enabled")).and_then(Json::as_bool)==Some(false){
        tracing::warn!("OpenClaw EMA plugin explicitly disabled; not overriding");
        return Ok(());
    }
    let entry=entries.entry("ema-report").or_insert_with(||json!({}));
    let Some(entry)=entry.as_object_mut() else{return Ok(());};
    entry.insert("enabled".into(),json!(true));
    // Required by upstream for agent_end. No prompt rewriting or tool access.
    let policy=entry.entry("hooks").or_insert_with(||json!({}));
    let Some(policy)=policy.as_object_mut() else{return Ok(());};
    if policy.get("allowConversationAccess").and_then(Json::as_bool)==Some(false){return Ok(());}
    policy.insert("allowConversationAccess".into(),json!(true));
    let new=serde_json::to_vec_pretty(&doc)?;
    if new!=before.as_bytes(){
        backup_once(&config,before.as_bytes())?;
        atomic_write(&config,&new)?;
    }
    Ok(())
}


fn install_dsh(home:&Path,shim:&Path)->Result<()> {
    let root=std::env::var_os("DSH_HOME").map(PathBuf::from)
        .unwrap_or_else(||home.join(".dsh"));
    let profiles=root.join("profiles");
    if !profiles.is_dir(){return Ok(());}
    let hook_script=shim.parent().unwrap_or(Path::new(".")).join("dsh-report.mjs");
    let js=r###"// EMA-MANAGED-HOOK: use dsh-hooks documented process-local event variables.
import { spawn } from "node:child_process";
const emitter = __SCRIPT__;
const env = process.env;
const reason = env.DSH_HOOK_REASON ?? "unknown";
if (env.DSH_HOOK_EVENT === "turn/end" && env.DSH_HOOK_SESSION_ID &&
    Number(env.DSH_HOOK_RUNNING_SUBAGENTS ?? "0") === 0) {
  const payload = JSON.stringify({
    type: "turn/end",
    session_id: env.DSH_HOOK_SESSION_ID,
    turn_id: env.DSH_HOOK_TURN ?? "",
    event_id: env.DSH_HOOK_SESSION_ID + ":" + (env.DSH_HOOK_TURN ?? ""),
    cwd: env.DSH_HOOK_CWD ?? "",
    response: (env.DSH_HOOK_CONTENT ?? "").slice(0,2000),
    completed: reason === "completed",
    failed: reason !== "completed",
    interrupted: reason === "aborted" || reason === "interrupted"
  });
  try {
    const p = spawn(emitter, ["dsh"], { stdio: ["pipe","ignore","ignore"] });
    p.on("error", () => {});
    p.stdin.on("error", () => {});
    p.stdin.end(payload);
  } catch (_) {}
}
"###.replace("__SCRIPT__",&serde_json::to_string(&shim.to_string_lossy().to_string())?);
    owned_file(&hook_script,&js)?;
    let command=format!("node {}",shell_quote(&hook_script.to_string_lossy()));
    // EMA only extends profiles with dsh-hooks already installed.
    for entry in fs::read_dir(profiles)?.flatten().take(64){
        if !entry.file_type().map(|x|x.is_dir()).unwrap_or(false){continue;}
        let file=entry.path().join("cordis.patch.yml");
        if !file.is_file(){continue;}
        let before=fs::read_to_string(&file)?;
        let mut doc:serde_yaml::Value=match serde_yaml::from_str(&before){
            Ok(x)=>x,Err(err)=>{
                tracing::warn!(file=%file.display(),"DSH profile invalid; preserving it: {err}");
                continue;
            }
        };
        let Some(rows)=doc.as_sequence_mut() else {continue;};
        let Some(row)=rows.iter_mut().find(|row|
            row.as_mapping().and_then(|m|m.get(&yaml_key("id")))
                .and_then(serde_yaml::Value::as_str)==Some("dsh-hooks")
        )else{continue;};
        let Some(record)=row.as_mapping_mut()else{continue;};
        if !record.contains_key(&yaml_key("config")){
            record.insert(yaml_key("config"),serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
        }
        let Some(config)=record.get_mut(&yaml_key("config"))
            .and_then(serde_yaml::Value::as_mapping_mut) else {continue;};
        if !config.contains_key(&yaml_key("hooks")){
            config.insert(yaml_key("hooks"),serde_yaml::Value::Sequence(Vec::new()));
        }
        let Some(hooks)=config.get_mut(&yaml_key("hooks"))
            .and_then(serde_yaml::Value::as_sequence_mut)else{continue;};
        if hooks.iter().any(|hook|hook.as_mapping()
            .and_then(|m|m.get(&yaml_key("run")))
            .and_then(serde_yaml::Value::as_str)==Some(command.as_str())){continue;}
        let mut rule=serde_yaml::Mapping::new();
        rule.insert(yaml_key("on"),yaml_key("turn/end"));
        rule.insert(yaml_key("run"),yaml_key(&command));
        rule.insert(yaml_key("timeoutMs"),serde_yaml::to_value(5000)?);
        hooks.push(serde_yaml::Value::Mapping(rule));
        let new=serde_yaml::to_string(&doc)?;
        if new!=before{
            backup_once(&file,before.as_bytes())?;
            atomic_write(&file,new.as_bytes())?;
        }
    }
    Ok(())
}

/// Reconcile only for agents with existing user profile directories.
pub fn install(home:&Path,shim:&Path) {
    for (agent,result) in [
        ("hermes",install_hermes(home,shim)),
        ("openclaw",install_openclaw(home,shim)),
        ("dsh",install_dsh(home,shim))
    ] {
        if let Err(err)=result{tracing::warn!(agent,"native Hook adapter not installed: {err:#}");}
    }
}
