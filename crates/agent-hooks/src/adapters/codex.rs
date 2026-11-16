//! Codex CLI: TOML `notify` slot in `~/.codex/config.toml`.
//! Codex appends the JSON payload as the final argv argument; stdin is unused.
use super::normalize;
use crate::adapter::{AgentAdapter, HookStatus, InstallOutcome};
use crate::event::NormalizedEvent;
use anyhow::Result;
use serde_json::Value;
use std::fs;
use std::path::Path;

pub struct Codex;

fn target(shim: &Path) -> String {
    shim.to_string_lossy().to_string()
}

impl AgentAdapter for Codex {
    fn name(&self) -> &'static str {
        "codex"
    }
    fn wire_events(&self) -> &[&'static str] {
        &["agent-turn-complete"]
    }
    fn normalize(&self, raw: &Value) -> Option<NormalizedEvent> {
        normalize("codex", raw, self.wire_events())
    }

    #[cfg(unix)]
    fn install(&self, home: &Path, shim: &Path) -> Result<InstallOutcome> {
        let dir = home.join(".codex");
        if !dir.is_dir() {
            return Ok(InstallOutcome::Skipped);
        }
        let file = dir.join("config.toml");
        let target = target(shim);
        let acted = std::cell::Cell::new(false);
        crate::util::surgical_write(&file, &|current| {
            let text = std::str::from_utf8(current)
                .map_err(|_| anyhow::anyhow!("Codex config is not valid UTF-8; preserving"))?;
            let mut doc: toml_edit::DocumentMut = text
                .parse()
                .map_err(|e| anyhow::anyhow!("Codex config unparseable, preserving: {e}"))?;
            // notify is a single slot. Already ours -> no-op; someone else's -> preserve.
            if let Some(existing) = doc.get("notify") {
                let owned = existing
                    .as_array()
                    .map(|a| {
                        a.get(0).and_then(|v| v.as_str()) == Some(target.as_str())
                            && a.get(1).and_then(|v| v.as_str()) == Some("codex")
                    })
                    .unwrap_or(false);
                if !owned {
                    tracing::warn!("Codex notify is already assigned; preserving user's notifier");
                }
                return Ok(None);
            }
            let mut array = toml_edit::Array::new();
            array.push(target.clone());
            array.push("codex");
            doc["notify"] = toml_edit::Item::Value(toml_edit::Value::Array(array));
            acted.set(true);
            Ok(Some(doc.to_string().into_bytes()))
        })?;
        Ok(if acted.get() { InstallOutcome::Synced } else { InstallOutcome::Skipped })
    }

    #[cfg(unix)]
    fn verify(&self, home: &Path, shim: &Path) -> HookStatus {
        let dir = home.join(".codex");
        if !dir.is_dir() {
            return HookStatus::Absent;
        }
        let file = dir.join("config.toml");
        let Ok(old) = fs::read_to_string(&file) else {
            return HookStatus::Drifted;
        };
        let Ok(doc) = old.parse::<toml_edit::DocumentMut>() else {
            return HookStatus::Unreadable;
        };
        let Some(current) = doc.get("notify") else {
            return HookStatus::Drifted;
        };
        let target = target(shim);
        let owned = current
            .as_array()
            .map(|a| {
                a.get(0).and_then(|v| v.as_str()) == Some(target.as_str())
                    && a.get(1).and_then(|v| v.as_str()) == Some("codex")
            })
            .unwrap_or(false);
        if owned {
            HookStatus::Installed
        } else {
            HookStatus::Foreign
        }
    }

    #[cfg(unix)]
    fn uninstall(&self, home: &Path, shim: &Path) -> Result<()> {
        let file = home.join(".codex/config.toml");
        if !file.is_file() {
            return Ok(());
        }
        let target = target(shim);
        crate::util::surgical_write(&file, &|current| {
            let text = std::str::from_utf8(current).map_err(|_| anyhow::anyhow!("not UTF-8"))?;
            let mut doc: toml_edit::DocumentMut = text.parse().map_err(|e| anyhow::anyhow!("{e}"))?;
            let Some(existing) = doc.get("notify") else {
                return Ok(None);
            };
            let ours = existing
                .as_array()
                .map(|a| {
                    a.get(0).and_then(|v| v.as_str()) == Some(target.as_str())
                        && a.get(1).and_then(|v| v.as_str()) == Some("codex")
                })
                .unwrap_or(false);
            if !ours {
                return Ok(None);
            }
            let _ = doc.as_table_mut().remove("notify");
            Ok(Some(doc.to_string().into_bytes()))
        })?;
        Ok(())
    }
}

#[cfg(unix)]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_maps_codex_notify_shape() {
        let raw: Value = serde_json::from_str(
            r#"{"type":"agent-turn-complete","thread-id":"t-1","turn-id":"5",
                "cwd":"/w","input-messages":["do it"],"prompt_response":"done"}"#,
        )
        .unwrap();
        let ev = Codex.normalize(&raw).unwrap();
        assert_eq!(ev.session_id, "t-1");
        assert_eq!(ev.event_id, "5");
        assert_eq!(ev.prompt, "", "input-messages is the argv echo and must not leak into reports");
        assert_eq!(ev.response, "done");
        assert_eq!(ev.status, crate::event::EventStatus::Completed);
        assert!(Codex.normalize(&serde_json::json!({"type":"other"})).is_none());
    }
}
