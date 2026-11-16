//! Kimi Code: TOML `[[hooks]]` tables in `~/.kimi-code/config.toml`
//! (older kimi-cli: `~/.kimi`). Stop and StopFailure.
use super::normalize;
use crate::adapter::{AgentAdapter, HookStatus, InstallOutcome};
use crate::event::NormalizedEvent;
use crate::util::surgical_write;
use anyhow::Result;
use serde_json::Value;
use std::fs;
use std::path::Path;

pub struct Kimi;

fn dir_of(home: &Path) -> std::path::PathBuf {
    if home.join(".kimi-code").is_dir() {
        home.join(".kimi-code")
    } else {
        home.join(".kimi")
    }
}

fn command(shim: &Path) -> String {
    format!("{} kimi", crate::util::shell_quote(&shim.to_string_lossy()))
}

impl AgentAdapter for Kimi {
    fn name(&self) -> &'static str {
        "kimi"
    }
    fn wire_events(&self) -> &[&'static str] {
        &["Stop", "StopFailure"]
    }
    fn normalize(&self, raw: &Value) -> Option<NormalizedEvent> {
        normalize("kimi", raw, self.wire_events())
    }

    #[cfg(unix)]
    fn install(&self, home: &Path, shim: &Path) -> Result<InstallOutcome> {
        let dir = dir_of(home);
        if !dir.is_dir() {
            return Ok(InstallOutcome::Skipped);
        }
        let file = dir.join("config.toml");
        let command = command(shim);
        let changed = std::cell::Cell::new(false);
        surgical_write(&file, &|current| {
            let text = std::str::from_utf8(current)
                .map_err(|_| anyhow::anyhow!("Kimi config is not valid UTF-8; preserving"))?;
            let mut doc: toml_edit::DocumentMut = text
                .parse()
                .map_err(|e| anyhow::anyhow!("Kimi config invalid, preserving it: {e}"))?;
            if doc.get("hooks").is_none() {
                doc["hooks"] = toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new());
            }
            let Some(hooks) = doc
                .get_mut("hooks")
                .and_then(toml_edit::Item::as_array_of_tables_mut)
            else {
                return Ok(None);
            };
            for event in ["Stop", "StopFailure"] {
                if hooks.iter().any(|t| {
                    t.get("event").and_then(|v| v.as_str()) == Some(event)
                        && t.get("command").and_then(|v| v.as_str()) == Some(command.as_str())
                }) {
                    continue;
                }
                let mut table = toml_edit::Table::new();
                table["event"] = toml_edit::value(event);
                table["command"] = toml_edit::value(command.as_str());
                table["timeout"] = toml_edit::value(5);
                hooks.push(table);
                changed.set(true);
            }
            if !changed.get() {
                return Ok(None);
            }
            Ok(Some(doc.to_string().into_bytes()))
        })?;
        Ok(if changed.get() { InstallOutcome::Synced } else { InstallOutcome::Skipped })
    }

    #[cfg(unix)]
    fn verify(&self, home: &Path, shim: &Path) -> HookStatus {
        let dir = dir_of(home);
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
        let Some(hooks) = doc.get("hooks").and_then(toml_edit::Item::as_array_of_tables) else {
            return HookStatus::Drifted;
        };
        let command = command(shim);
        let has = |event: &str| {
            hooks.iter().any(|t| {
                t.get("event").and_then(|v| v.as_str()) == Some(event)
                    && t.get("command").and_then(|v| v.as_str()) == Some(command.as_str())
            })
        };
        match (has("Stop"), has("StopFailure")) {
            (true, true) => HookStatus::Installed,
            _ => HookStatus::Drifted,
        }
    }

    #[cfg(unix)]
    fn uninstall(&self, home: &Path, shim: &Path) -> Result<()> {
        let file = dir_of(home).join("config.toml");
        if !file.is_file() {
            return Ok(());
        }
        let command = command(shim);
        surgical_write(&file, &|current| {
            let Some(mut doc) = std::str::from_utf8(current)
                .ok()
                .and_then(|t| t.parse::<toml_edit::DocumentMut>().ok())
            else {
                return Ok(None);
            };
            if let Some(hooks) = doc
                .get_mut("hooks")
                .and_then(toml_edit::Item::as_array_of_tables_mut)
            {
                hooks.retain(|t| {
                    t.get("command").and_then(|v| v.as_str()) != Some(command.as_str())
                });
            }
            Ok(Some(doc.to_string().into_bytes()))
        })?;
        Ok(())
    }
}
