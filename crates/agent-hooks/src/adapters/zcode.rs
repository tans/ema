//! ZCode: user hooks exclusively from `~/.zcode/cli/config.json`.
//! An explicitly disabled hook collection is never re-enabled.
use super::normalize;
use crate::adapter::{AgentAdapter, HookStatus, InstallOutcome};
use crate::event::NormalizedEvent;
use crate::util::surgical_write;
use anyhow::Result;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

pub struct Zcode;

fn executable(shim: &Path) -> String {
    shim.to_string_lossy().to_string()
}

impl AgentAdapter for Zcode {
    fn name(&self) -> &'static str {
        "zcode"
    }
    fn wire_events(&self) -> &[&'static str] {
        &["Stop"]
    }
    fn normalize(&self, raw: &Value) -> Option<NormalizedEvent> {
        normalize("zcode", raw, self.wire_events())
    }

    #[cfg(unix)]
    fn install(&self, home: &Path, shim: &Path) -> Result<InstallOutcome> {
        let folder = home.join(".zcode/cli");
        if !folder.is_dir() {
            return Ok(InstallOutcome::Skipped);
        }
        let file = folder.join("config.json");
        let executable = executable(shim);
        let changed = std::cell::Cell::new(false);
        surgical_write(&file, &|current| {
            let mut document: Value = if current.is_empty() {
                json!({})
            } else {
                serde_json::from_slice(current)
                    .map_err(|err| anyhow::anyhow!("ZCode config invalid, preserving it: {err}"))?
            };
            let Some(root) = document.as_object_mut() else {
                return Ok(None);
            };
            let hooks = root.entry("hooks").or_insert_with(|| json!({}));
            let Some(hooks) = hooks.as_object_mut() else {
                tracing::warn!("ZCode hooks config is incompatible; preserving existing settings");
                return Ok(None);
            };
            // Do not enable a previously explicitly-disabled global hook collection.
            if hooks.get("enabled").and_then(Value::as_bool) == Some(false) {
                return Ok(None);
            }
            hooks.insert("enabled".into(), json!(true));
            let events = hooks.entry("events").or_insert_with(|| json!({}));
            let Some(events) = events.as_object_mut() else {
                return Ok(None);
            };
            let stop = events.entry("Stop").or_insert_with(|| json!([]));
            let Some(groups) = stop.as_array_mut() else {
                return Ok(None);
            };
            let already_present = groups.iter().any(|group| {
                group
                    .get("hooks")
                    .and_then(Value::as_array)
                    .map(|hooks| {
                        hooks.iter().any(|entry| {
                            entry.get("command").and_then(Value::as_str) == Some(executable.as_str())
                                && entry
                                    .get("args")
                                    .and_then(Value::as_array)
                                    .and_then(|arr| arr.first())
                                    .and_then(Value::as_str)
                                    == Some("zcode")
                        })
                    })
                    .unwrap_or(false)
            });
            if !already_present {
                changed.set(true);
                groups.push(json!({
                    "hooks":[{
                        "type":"process","command":executable,
                        "args":["zcode"],"enabled":true,"timeoutMs":5000
                    }]
                }));
            }
            let updated = serde_json::to_vec_pretty(&document)?;
            if updated == current {
                return Ok(None);
            }
            Ok(Some(updated))
        })?;
        Ok(if changed.get() { InstallOutcome::Synced } else { InstallOutcome::Skipped })
    }

    #[cfg(unix)]
    fn verify(&self, home: &Path, shim: &Path) -> HookStatus {
        let folder = home.join(".zcode/cli");
        if !folder.is_dir() {
            return HookStatus::Absent;
        }
        let file = folder.join("config.json");
        let Ok(old) = fs::read_to_string(&file) else {
            return HookStatus::Drifted;
        };
        let Ok(doc) = serde_json::from_str::<Value>(&old) else {
            return HookStatus::Unreadable;
        };
        if doc["hooks"]["enabled"].as_bool() == Some(false) {
            return HookStatus::Denied;
        }
        let executable = executable(shim);
        let installed = doc["hooks"]["events"]["Stop"].as_array().into_iter().flatten()
            .flat_map(|g| g["hooks"].as_array().into_iter().flatten())
            .any(|entry| {
                entry.get("command").and_then(Value::as_str) == Some(executable.as_str())
                    && entry
                        .get("args")
                        .and_then(Value::as_array)
                        .and_then(|arr| arr.first())
                        .and_then(Value::as_str)
                        == Some("zcode")
            });
        if installed { HookStatus::Installed } else { HookStatus::Drifted }
    }

    #[cfg(unix)]
    fn uninstall(&self, home: &Path, shim: &Path) -> Result<()> {
        let file = home.join(".zcode/cli/config.json");
        if !file.is_file() {
            return Ok(());
        }
        let executable = executable(shim);
        surgical_write(&file, &|current| {
            let Ok(mut doc) = serde_json::from_slice::<Value>(current) else {
                return Ok(None);
            };
            if let Some(groups) = doc["hooks"]["events"]["Stop"].as_array_mut() {
                for group in groups.iter_mut() {
                    if let Some(hooks) = group["hooks"].as_array_mut() {
                        hooks.retain(|entry| {
                            entry.get("command").and_then(Value::as_str) != Some(executable.as_str())
                        });
                    }
                }
                groups.retain(|g| g["hooks"].as_array().map(|a| !a.is_empty()).unwrap_or(true));
            }
            Ok(Some(serde_json::to_vec_pretty(&doc)?))
        })?;
        Ok(())
    }
}
