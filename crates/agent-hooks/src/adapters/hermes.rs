//! Hermes: Python plugin at `~/.hermes/plugins/ema-report`, enabled via
//! `plugins.enabled` in `~/.hermes/config.yaml`.
use super::normalize;
use crate::adapter::{AgentAdapter, HookStatus, InstallOutcome};
use crate::event::NormalizedEvent;
use crate::util::{owned_file, surgical_write};
use anyhow::Result;
use serde_json::Value;
use serde_yaml::Value as Yv;
use std::fs;
use std::path::{Path, PathBuf};

pub struct Hermes;

fn plugin_dir(home: &Path) -> PathBuf {
    home.join(".hermes/plugins/ema-report")
}

fn manifest_text() -> &'static str {
    "# EMA-MANAGED-HOOK\nname: ema-report\nversion: \"1.0\"\ndescription: EMA local turn status emails\nprovides_hooks:\n  - on_session_end\n"
}

fn plugin_code(shim: &Path) -> String {
    // EMA-MANAGED-HOOK: lightweight Hermes lifecycle observer.
    const TEMPLATE: &str = r###"# EMA-MANAGED-HOOK: lightweight Hermes lifecycle observer.
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
"###;
    TEMPLATE.replace("__SCRIPT__", &serde_json::to_string(&shim.to_string_lossy().to_string()).unwrap())
}

impl AgentAdapter for Hermes {
    fn name(&self) -> &'static str {
        "hermes"
    }
    fn wire_events(&self) -> &[&'static str] {
        &["on_session_end"]
    }
    fn normalize(&self, raw: &Value) -> Option<NormalizedEvent> {
        normalize("hermes", raw, self.wire_events())
    }

    #[cfg(unix)]
    fn install(&self, home: &Path, shim: &Path) -> Result<InstallOutcome> {
        let root = home.join(".hermes");
        if !root.is_dir() {
            return Ok(InstallOutcome::Skipped);
        }
        let plugin = plugin_dir(home);
        let manifest = plugin.join("plugin.yaml");
        if manifest.exists()
            && !fs::read_to_string(&manifest)?.contains(crate::util::MARKER)
        {
            tracing::warn!("Hermes ema-report plugin belongs to another owner; skipping");
            return Ok(InstallOutcome::Skipped);
        }
        let _ = owned_file(&manifest, manifest_text())?;
        let _ = owned_file(&plugin.join("__init__.py"), &plugin_code(shim))?;
        // Only install in user-owned Hermes, not in project-local directories.
        let conf = root.join("config.yaml");
        surgical_write(&conf, &|current| {
            let before = std::str::from_utf8(current)
                .map_err(|_| anyhow::anyhow!("Hermes config is not valid UTF-8"))?;
            let mut doc: Yv = if before.trim().is_empty() {
                Yv::Mapping(serde_yaml::Mapping::new())
            } else {
                serde_yaml::from_str(before)
                    .map_err(|err| anyhow::anyhow!("Hermes config invalid; do not enable plugin: {err}"))?
            };
            let Some(root_map) = doc.as_mapping_mut() else {
                return Ok(None);
            };
            if !root_map.contains_key(&Yv::String("plugins".into())) {
                root_map.insert(
                    Yv::String("plugins".into()),
                    Yv::Mapping(serde_yaml::Mapping::new()),
                );
            }
            let Some(plugins) = root_map
                .get_mut(&Yv::String("plugins".into()))
                .and_then(Yv::as_mapping_mut)
            else {
                return Ok(None);
            };
            let disabled = plugins
                .get(&Yv::String("disabled".into()))
                .and_then(Yv::as_sequence)
                .map(|arr| arr.iter().any(|v| v.as_str() == Some("ema-report")))
                .unwrap_or(false);
            if disabled {
                tracing::warn!("Hermes plugin explicitly disabled; respecting user's choice");
                return Ok(None);
            }
            if !plugins.contains_key(&Yv::String("enabled".into())) {
                plugins.insert(Yv::String("enabled".into()), Yv::Sequence(Vec::new()));
            }
            let Some(enabled) = plugins
                .get_mut(&Yv::String("enabled".into()))
                .and_then(Yv::as_sequence_mut)
            else {
                return Ok(None);
            };
            if enabled.iter().any(|v| v.as_str() == Some("ema-report")) {
                return Ok(None);
            }
            enabled.push(Yv::String("ema-report".into()));
            let new = serde_yaml::to_string(&doc)?;
            // YAML is semantic-preserving but may normalize formatting; retain source backup.
            if new == before {
                return Ok(None);
            }
            Ok(Some(new.into_bytes()))
        })?;
        Ok(InstallOutcome::Synced)
    }

    #[cfg(unix)]
    fn verify(&self, home: &Path, _shim: &Path) -> HookStatus {
        let root = home.join(".hermes");
        if !root.is_dir() {
            return HookStatus::Absent;
        }
        let conf = root.join("config.yaml");
        let Ok(old) = fs::read_to_string(&conf) else {
            return HookStatus::Drifted;
        };
        let Ok(doc) = serde_yaml::from_str::<Yv>(&old) else {
            return HookStatus::Unreadable;
        };
        let plugins = doc.get(&Yv::String("plugins".into()));
        let list = |key: &str| {
            plugins
                .and_then(|p| p.get(&Yv::String(key.into())))
                .and_then(Yv::as_sequence)
                .map(|arr| arr.iter().any(|v| v.as_str() == Some("ema-report")))
                .unwrap_or(false)
        };
        if list("disabled") {
            HookStatus::Denied
        } else if list("enabled") {
            HookStatus::Installed
        } else {
            HookStatus::Drifted
        }
    }

    #[cfg(unix)]
    fn uninstall(&self, home: &Path, _shim: &Path) -> Result<()> {
        let root = home.join(".hermes");
        if !root.is_dir() {
            return Ok(());
        }
        let plugin = plugin_dir(home);
        let manifest = plugin.join("plugin.yaml");
        if manifest.is_file() && crate::util::is_managed_file(&manifest) {
            let _ = fs::remove_dir_all(&plugin);
        }
        let conf = root.join("config.yaml");
        if !conf.is_file() {
            return Ok(());
        }
        surgical_write(&conf, &|current| {
            let Ok(old) = std::str::from_utf8(current) else {
                return Ok(None);
            };
            let Ok(mut doc) = serde_yaml::from_str::<Yv>(old) else {
                return Ok(None);
            };
            if let Some(mapping) = doc.as_mapping_mut() {
                let _ = mapping
                    .get_mut(&Yv::String("plugins".into()))
                    .and_then(Yv::as_mapping_mut)
                    .and_then(|p| p.get_mut(&Yv::String("enabled".into())))
                    .and_then(Yv::as_sequence_mut)
                    .map(|enabled| enabled.retain(|v| v.as_str() != Some("ema-report")));
            }
            Ok(Some(serde_yaml::to_string(&doc)?.into_bytes()))
        })?;
        Ok(())
    }
}
