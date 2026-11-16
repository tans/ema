//! Gemini CLI: `hooks.AfterAgent` in `~/.gemini/settings.json`.
use super::normalize;
use crate::adapter::{AgentAdapter, HookStatus, InstallOutcome};
use crate::event::NormalizedEvent;
use crate::util::json_hook;
use anyhow::Result;
use serde_json::{Value, json};
use std::fs;
use std::path::Path;

pub struct Gemini;

fn command(shim: &Path) -> String {
    format!("{} gemini", crate::util::shell_quote(&shim.to_string_lossy()))
}

fn hook_value(shim: &Path) -> Value {
    json!({"hooks":[{"name":"ema-report","type":"command",
        "command":command(shim),"timeout":5000}]})
}

impl AgentAdapter for Gemini {
    fn name(&self) -> &'static str {
        "gemini"
    }
    fn wire_events(&self) -> &[&'static str] {
        &["AfterAgent"]
    }
    fn normalize(&self, raw: &Value) -> Option<NormalizedEvent> {
        normalize("gemini", raw, self.wire_events())
    }

    #[cfg(unix)]
    fn install(&self, home: &Path, shim: &Path) -> Result<InstallOutcome> {
        let dir = home.join(".gemini");
        if !dir.is_dir() {
            return Ok(InstallOutcome::Skipped);
        }
        let wrote = json_hook(&dir.join("settings.json"), "AfterAgent", &hook_value(shim))?;
        Ok(if wrote { InstallOutcome::Synced } else { InstallOutcome::Skipped })
    }

    #[cfg(unix)]
    fn verify(&self, home: &Path, shim: &Path) -> HookStatus {
        let file = home.join(".gemini/settings.json");
        if !home.join(".gemini").is_dir() {
            return HookStatus::Absent;
        }
        let Ok(old) = fs::read_to_string(&file) else {
            return HookStatus::Drifted;
        };
        let Ok(doc) = serde_json::from_str::<Value>(&old) else {
            return HookStatus::Unreadable;
        };
        let target = command(shim);
        let installed = doc["hooks"]["AfterAgent"].as_array().into_iter().flatten()
            .flat_map(|g| g["hooks"].as_array().into_iter().flatten())
            .any(|h| h["name"].as_str() == Some("ema-report") && h["command"].as_str() == Some(target.as_str()));
        if installed { HookStatus::Installed } else { HookStatus::Drifted }
    }

    #[cfg(unix)]
    fn uninstall(&self, home: &Path, shim: &Path) -> Result<()> {
        let file = home.join(".gemini/settings.json");
        if !file.is_file() {
            return Ok(());
        }
        let target = command(shim);
        crate::util::surgical_write(&file, &|current| {
            let Ok(mut doc) = serde_json::from_slice::<Value>(current) else {
                return Ok(None);
            };
            if let Some(groups) = doc["hooks"]["AfterAgent"].as_array_mut() {
                for group in groups.iter_mut() {
                    if let Some(hooks) = group["hooks"].as_array_mut() {
                        hooks.retain(|h| h["command"].as_str() != Some(target.as_str()));
                    }
                }
                groups.retain(|g| g["hooks"].as_array().map(|a| !a.is_empty()).unwrap_or(true));
            }
            Ok(Some(serde_json::to_vec_pretty(&doc)?))
        })?;
        Ok(())
    }
}
