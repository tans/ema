//! Alma: command hook on `chat.message.didReceive` in `~/.config/alma/hooks.json`.
//! Alma wraps its hook payload in an envelope ({hook, matcher, input}), which
//! this adapter reduces to the canonical event. Explicit disables (the
//! `hooks/alma.disabled` marker in EMA's root) survive automatic reconciliation.
use crate::adapter::{AgentAdapter, HookStatus, InstallOutcome};
use crate::event::{EventStatus, NormalizedEvent};
use crate::util::{atomic_write, shell_quote, surgical_write};
use anyhow::Result;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

const EVENT: &str = "chat.message.didReceive";

pub struct Alma;

fn config_path(home: &Path) -> PathBuf {
    home.join(".config/alma/hooks.json")
}

fn root_of(shim: &Path) -> Option<PathBuf> {
    shim.parent().and_then(Path::parent).map(PathBuf::from)
}

fn disabled(root: &Path) -> PathBuf {
    root.join("hooks/alma.disabled")
}

fn command(shim: &Path) -> String {
    format!("{} alma", shell_quote(&shim.to_string_lossy()))
}

fn owned(entry: &Value, target: &str) -> bool {
    entry["type"] == "command" && entry["command"].as_str() == Some(target)
}

/// Automatic reconciliation for existing Alma profiles only; explicit
/// disables survive. Returns whether a change was written.
pub(crate) fn register_config(home: &Path, shim: &Path) -> Result<InstallOutcome> {
    let file = config_path(home);
    let target = command(shim);
    let changed = std::cell::Cell::new(false);
    surgical_write(&file, &|current| {
        let mut doc: Value = if current.is_empty() {
            json!({})
        } else {
            serde_json::from_slice(current)
                .map_err(|e| anyhow::anyhow!("invalid Alma hooks JSON, preserving file: {e}"))?
        };
        let Some(items) = doc
            .get_mut("hooks")
            .and_then(Value::as_object_mut)
            .and_then(|hooks| {
                hooks
                    .entry(EVENT)
                    .or_insert_with(|| json!([]))
                    .as_array_mut()
            })
        else {
            return Ok(None);
        };
        if items
            .iter()
            .any(|g| g["hooks"].as_array().map(|a| a.iter().any(|h| owned(h, &target))).unwrap_or(false))
        {
            return Ok(None);
        }
        items.push(json!({"matcher":".*","hooks":[{
            "type":"command","command":target,"timeout":3000,"enabled":true
        }]}));
        changed.set(true);
        Ok(Some(serde_json::to_vec_pretty(&doc)?))
    })?;
    Ok(if changed.get() { InstallOutcome::Synced } else { InstallOutcome::Skipped })
}

fn text(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_owned();
    }
    v.as_array()
        .into_iter()
        .flatten()
        .filter(|p| p["type"] == "text")
        .filter_map(|p| p["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

impl AgentAdapter for Alma {
    fn name(&self) -> &'static str {
        "alma"
    }
    fn wire_events(&self) -> &[&'static str] {
        &[EVENT]
    }

    /// Reduce the Alma envelope to the canonical event. Never inspects transcripts.
    fn normalize(&self, v: &Value) -> Option<NormalizedEvent> {
        if v["hook"] != EVENT {
            return None;
        }
        let input = v.get("input")?;
        let session = input["threadId"]
            .as_str()
            .filter(|s| !s.trim().is_empty())?;
        let id = input["message"]["id"]
            .as_str()
            .filter(|s| !s.trim().is_empty())?;
        let mut response = text(&input["response"]["content"]);
        if response.is_empty() && input["message"]["role"] == "assistant" {
            response = text(&input["message"]["parts"]);
        }
        Some(NormalizedEvent {
            agent: "alma",
            event_type: EVENT.to_owned(),
            session_id: session.to_owned(),
            event_id: id.to_owned(),
            cwd: input["cwd"].as_str().unwrap_or("").to_owned(),
            title: String::new(),
            prompt: String::new(),
            response,
            context_messages: Vec::new(),
            status: EventStatus::Completed,
            prompt_truncated: false,
            response_truncated: false,
            raw: v.clone(),
        })
    }

    #[cfg(unix)]
    fn install(&self, home: &Path, shim: &Path) -> Result<InstallOutcome> {
        let Some(root) = root_of(shim) else {
            return Ok(InstallOutcome::Skipped);
        };
        if disabled(&root).exists() || !home.join(".config/alma").is_dir() {
            return Ok(InstallOutcome::Skipped);
        }
        register_config(home, shim)
    }

    #[cfg(unix)]
    fn verify(&self, home: &Path, shim: &Path) -> HookStatus {
        let Some(root) = root_of(shim) else {
            return HookStatus::Absent;
        };
        if disabled(&root).exists() {
            return HookStatus::Denied;
        }
        if !home.join(".config/alma").is_dir() {
            return HookStatus::Absent;
        }
        let file = config_path(home);
        let Ok(old) = fs::read_to_string(&file) else {
            return HookStatus::Drifted;
        };
        let Ok(doc) = serde_json::from_str::<Value>(&old) else {
            return HookStatus::Unreadable;
        };
        let target = command(shim);
        let entries: Vec<&Value> = doc["hooks"][EVENT]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|g| g["hooks"].as_array().into_iter().flatten())
            .filter(|h| owned(h, &target))
            .collect();
        if entries.is_empty() {
            return HookStatus::Drifted;
        }
        // An explicitly disabled EMA entry is the owner's choice.
        let enabled = entries.iter().all(|h| h["enabled"].as_bool() != Some(false));
        if enabled {
            HookStatus::Installed
        } else {
            HookStatus::Denied
        }
    }

    #[cfg(unix)]
    fn uninstall(&self, home: &Path, shim: &Path) -> Result<()> {
        let file = config_path(home);
        let target = command(shim);
        surgical_write(&file, &|current| {
            let Ok(mut doc) = serde_json::from_slice::<Value>(current) else {
                return Ok(None);
            };
            if let Some(items) = doc
                .get_mut("hooks")
                .and_then(Value::as_object_mut)
                .and_then(|hooks| hooks.get_mut(EVENT))
                .and_then(Value::as_array_mut)
            {
                items.retain_mut(|g| {
                    let Some(hooks) = g.get_mut("hooks").and_then(Value::as_array_mut) else {
                        return true;
                    };
                    let before = hooks.len();
                    hooks.retain(|h| !owned(h, &target));
                    before == hooks.len() || !hooks.is_empty()
                });
            }
            Ok(Some(serde_json::to_vec_pretty(&doc)?))
        })?;
        if let Some(root) = root_of(shim) {
            // Prevent the running gateway from reinstalling on its next reconciliation.
            atomic_write(
                &disabled(&root),
                b"EMA-MANAGED-HOOK: Alma explicitly uninstalled\n",
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> Value {
        json!({"hook":"chat.message.didReceive","matcher":".*","input":{
            "threadId":"alma-thread", "message":{"id":"reply-1","role":"assistant",
                "parts":[{"type":"text","text":"fallback"}]},
            "response":{"content":"Alma final reply", "usage":{"outputTokens":12}}
        }})
    }

    #[test]
    fn normalizes_main_reply_only() {
        let ev = Alma.normalize(&payload()).expect("Alma main reply");
        assert_eq!(ev.session_id, "alma-thread");
        assert_eq!(ev.event_id, "reply-1");
        assert_eq!(ev.response, "Alma final reply");

        let mut parts = payload();
        parts["input"]["response"]["content"] = json!([
            {"type":"reasoning","text":"private thought"},
            {"type":"text","text":"visible reply"}]);
        assert_eq!(Alma.normalize(&parts).unwrap().response, "visible reply");

        for event in ["chat.message.willSend", "subagent.didComplete"] {
            let mut other = payload();
            other["hook"] = json!(event);
            assert!(Alma.normalize(&other).is_none());
        }

        let mut fallback = payload();
        fallback["input"]["response"] = Value::Null;
        assert_eq!(Alma.normalize(&fallback).unwrap().response, "fallback");
        fallback["input"]["message"]["role"] = json!("user");
        assert_eq!(Alma.normalize(&fallback).unwrap().response, "");

        let mut missing = payload();
        missing["input"]["threadId"] = json!("");
        assert!(Alma.normalize(&missing).is_none());
    }
}
