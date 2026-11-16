//! OpenClaw: TS extension at `~/.openclaw/extensions/ema-report`, enabled via
//! `plugins.entries.ema-report` in `~/.openclaw/openclaw.json` (JSON5).
use super::normalize;
use crate::adapter::{AgentAdapter, HookStatus, InstallOutcome};
use crate::event::NormalizedEvent;
use crate::util::{owned_file, surgical_write};
use anyhow::Result;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

pub struct Openclaw;

fn plugin_dir(home: &Path) -> PathBuf {
    home.join(".openclaw/extensions/ema-report")
}

fn module_source(shim: &Path) -> String {
    // EMA-MANAGED-HOOK: observe agent completion and pass a short final answer.
    const TEMPLATE: &str = r###"// EMA-MANAGED-HOOK: observe agent completion and pass a short final answer.
import { spawn } from "node:child_process";
const script = __SCRIPT__;
export default {
  id: "ema-report",
  name: "EMA Email Completion Reports",
  register(api) {
    api.on("agent_end", async (event, ctx) => {
      const messages = Array.isArray(event?.messages) ? event.messages : [];
      const last = messages.filter(m => m?.role === "assistant").at(-1);
      const input = messages.filter(m => m?.role === "user").at(-1);
      const prompt = typeof input?.content === "string" ? input.content
        : (Array.isArray(input?.content) ? input.content : []).filter(p => p?.type === "text").map(p => p.text).join("\n");
      let result = "";
      const content = last?.content;
      if (typeof content === "string") result = content;
      else if (Array.isArray(content)) {
        result = content.filter(p => p?.type === "text").map(p => p.text).join("\n");
      }
      const payload = JSON.stringify({
        type: "agent_end",
        session_id: ctx?.sessionKey ?? ctx?.sessionId ?? "",
        event_id: ctx?.runId ?? event?.runId ?? "",
        completed: !!event?.success,
        failed: event?.success === false,
        prompt: prompt.slice(0, 4000),
        response: result.slice(0, 10000),
        prompt_truncated: prompt.length > 4000, response_truncated: result.length > 10000
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
"###;
    TEMPLATE.replace("__SCRIPT__", &serde_json::to_string(&shim.to_string_lossy().to_string()).unwrap())
}

impl AgentAdapter for Openclaw {
    fn name(&self) -> &'static str {
        "openclaw"
    }
    fn wire_events(&self) -> &[&'static str] {
        &["agent_end"]
    }
    fn normalize(&self, raw: &Value) -> Option<NormalizedEvent> {
        normalize("openclaw", raw, self.wire_events())
    }

    #[cfg(unix)]
    fn install(&self, home: &Path, shim: &Path) -> Result<InstallOutcome> {
        let root = home.join(".openclaw");
        if !root.is_dir() {
            return Ok(InstallOutcome::Skipped);
        }
        let plugin = plugin_dir(home);
        let manifest = plugin.join("openclaw.plugin.json");
        if manifest.exists() && !fs::read_to_string(&manifest)?.contains(crate::util::MARKER) {
            tracing::warn!("OpenClaw ema-report plugin belongs to another owner; skipping");
            return Ok(InstallOutcome::Skipped);
        }
        let _ = owned_file(&manifest, &serde_json::to_string_pretty(&json!({"id": "ema-report","name": "EMA Email Completion Reports",
            "description": "EMA-MANAGED-HOOK, short completion reports to owner's mailbox",
            "activation":{"onStartup":true},
            "configSchema":{"type":"object","additionalProperties":false}}))?);
        let _ = owned_file(&plugin.join("package.json"), &serde_json::to_string_pretty(&json!({"name":"ema-report","version":"1.0.0","type":"module",
            "description":"EMA-MANAGED-HOOK: local agent completion observer",
            "openclaw":{"extensions":["./index.ts"]}}))?);
        let _ = owned_file(&plugin.join("index.ts"), &module_source(shim))?;
        let config = root.join("openclaw.json");
        let changed = std::cell::Cell::new(false);
        surgical_write(&config, &|current| {
            let mut doc: Value = if current.is_empty() || std::str::from_utf8(current).map(|s| s.trim().is_empty()).unwrap_or(true) {
                json!({})
            } else {
                json5::from_str(std::str::from_utf8(current)
                    .map_err(|e| anyhow::anyhow!("openclaw config is not UTF-8: {e}"))?)
                    .map_err(|err| anyhow::anyhow!("OpenClaw config incompatible; plugin installed but not enabled: {err}"))?
            };
            let Some(root) = doc.as_object_mut() else {
                return Ok(None);
            };
            let plugins = root.entry("plugins").or_insert_with(|| json!({}));
            let Some(plugins) = plugins.as_object_mut() else {
                return Ok(None);
            };
            if plugins.get("enabled").and_then(Value::as_bool) == Some(false) {
                return Ok(None);
            }
            if plugins.get("deny").and_then(Value::as_array).map(|a| a.iter().any(|v| v.as_str() == Some("ema-report"))).unwrap_or(false) {
                return Ok(None);
            }
            if let Some(allow) = plugins.get("allow").and_then(Value::as_array) {
                if !allow.iter().any(|v| v.as_str() == Some("ema-report")) {
                    // The owner has explicitly restricted plugin loading.
                    tracing::warn!("OpenClaw plugin allow-list excludes EMA; not overriding it");
                    return Ok(None);
                }
            }
            let entries = plugins.entry("entries").or_insert_with(|| json!({}));
            let Some(entries) = entries.as_object_mut() else {
                return Ok(None);
            };
            let entry = entries.entry("ema-report").or_insert_with(|| json!({}));
            let Some(entry) = entry.as_object_mut() else {
                return Ok(None);
            };
            if entry.get("enabled").and_then(Value::as_bool) == Some(false) {
                tracing::warn!("OpenClaw EMA plugin explicitly disabled; not overriding");
                return Ok(None);
            }
            entry.insert("enabled".into(), json!(true));
            // Required by upstream for agent_end. No prompt rewriting or tool access.
            let policy = entry.entry("hooks").or_insert_with(|| json!({}));
            let Some(policy) = policy.as_object_mut() else {
                return Ok(None);
            };
            if policy.get("allowConversationAccess").and_then(Value::as_bool) == Some(false) {
                return Ok(None);
            }
            policy.insert("allowConversationAccess".into(), json!(true));
            let new = serde_json::to_vec_pretty(&doc)?;
            if new == current {
                return Ok(None);
            }
            changed.set(true);
            Ok(Some(new))
        })?;
        Ok(if changed.get() { InstallOutcome::Synced } else { InstallOutcome::Skipped })
    }

    #[cfg(unix)]
    fn verify(&self, home: &Path, _shim: &Path) -> HookStatus {
        let root = home.join(".openclaw");
        if !root.is_dir() {
            return HookStatus::Absent;
        }
        let config = root.join("openclaw.json");
        let Ok(old) = fs::read_to_string(&config) else {
            return HookStatus::Drifted;
        };
        let Ok(doc) = json5::from_str::<Value>(&old) else {
            return HookStatus::Unreadable;
        };
        let plugins = doc.get("plugins");
        let list = |key: &str| {
            plugins
                .and_then(|p| p.get(key))
                .and_then(Value::as_array)
                .map(|arr| arr.iter().any(|v| v.as_str() == Some("ema-report")))
                .unwrap_or(false)
        };
        if plugins.and_then(|p| p.get("enabled")).and_then(Value::as_bool) == Some(false)
            || list("deny")
        {
            return HookStatus::Denied;
        }
        if let Some(allow) = plugins.and_then(|p| p.get("allow")).and_then(Value::as_array) {
            if !allow.iter().any(|v| v.as_str() == Some("ema-report")) {
                return HookStatus::Denied;
            }
        }
        let entry = plugins
            .and_then(|p| p.get("entries"))
            .and_then(|e| e.get("ema-report"));
        if entry.and_then(|e| e.get("enabled")).and_then(Value::as_bool) == Some(true)
            && entry
                .and_then(|e| e.get("hooks"))
                .and_then(|h| h.get("allowConversationAccess"))
                .and_then(Value::as_bool)
                != Some(false)
        {
            HookStatus::Installed
        } else {
            HookStatus::Drifted
        }
    }

    #[cfg(unix)]
    fn uninstall(&self, home: &Path, _shim: &Path) -> Result<()> {
        let root = home.join(".openclaw");
        if !root.is_dir() {
            return Ok(());
        }
        let plugin = plugin_dir(home);
        let manifest = plugin.join("openclaw.plugin.json");
        if manifest.is_file() && crate::util::is_managed_file(&manifest) {
            let _ = fs::remove_dir_all(&plugin);
        }
        let config = root.join("openclaw.json");
        if !config.is_file() {
            return Ok(());
        }
        surgical_write(&config, &|current| {
            let Ok(old) = std::str::from_utf8(current) else {
                return Ok(None);
            };
            let Ok(mut doc) = json5::from_str::<Value>(old) else {
                return Ok(None);
            };
            if let Some(entries) = doc["plugins"]["entries"].as_object_mut() {
                entries.remove("ema-report");
            }
            Ok(Some(serde_json::to_vec_pretty(&doc)?))
        })?;
        Ok(())
    }
}
