//! DSH: node hook script registered through `dsh-hooks` config in each
//! profile's `cordis.patch.yml`. EMA only extends profiles that already use
//! the dsh-hooks plugin; the payload comes from documented process-local
//! `DSH_HOOK_*` environment variables on `turn/end`.
use super::normalize;
use crate::adapter::{AgentAdapter, HookStatus, InstallOutcome};
use crate::event::NormalizedEvent;
use crate::util::{owned_file, shell_quote, surgical_write};
use anyhow::Result;
use serde_json::Value;
use serde_yaml::Value as Yv;
use std::path::{Path, PathBuf};

pub struct Dsh;

fn root_of(home: &Path) -> PathBuf {
    std::env::var_os("DSH_HOME").map(PathBuf::from).unwrap_or_else(|| home.join(".dsh"))
}

fn hook_script(shim: &Path) -> PathBuf {
    shim.parent().map(PathBuf::from).unwrap_or_default().join("dsh-report.mjs")
}

fn script_source(shim: &Path) -> String {
    // EMA-MANAGED-HOOK: use dsh-hooks documented process-local event variables.
    const TEMPLATE: &str = r###"// EMA-MANAGED-HOOK: use dsh-hooks documented process-local event variables.
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
"###;
    TEMPLATE.replace("__SCRIPT__", &serde_json::to_string(&shim.to_string_lossy().to_string()).unwrap())
}

fn run_command(shim: &Path) -> String {
    format!("node {}", shell_quote(&hook_script(shim).to_string_lossy()))
}

impl AgentAdapter for Dsh {
    fn name(&self) -> &'static str {
        "dsh"
    }
    fn wire_events(&self) -> &[&'static str] {
        &["turn/end"]
    }
    fn normalize(&self, raw: &Value) -> Option<NormalizedEvent> {
        normalize("dsh", raw, self.wire_events())
    }

    #[cfg(unix)]
    fn install(&self, home: &Path, shim: &Path) -> Result<InstallOutcome> {
        let root = root_of(home);
        let profiles = root.join("profiles");
        if !profiles.is_dir() {
            return Ok(InstallOutcome::Skipped);
        }
        let hook_script = hook_script(shim);
        let _ = owned_file(&hook_script, &script_source(shim))?;
        let command = run_command(shim);
        let mut changed = false;
        // EMA only extends profiles with dsh-hooks already installed.
        for entry in std::fs::read_dir(profiles)?.flatten().take(64) {
            if !entry.file_type().map(|x| x.is_dir()).unwrap_or(false) {
                continue;
            }
            let file = entry.path().join("cordis.patch.yml");
            if !file.is_file() {
                continue;
            }
            let touched = std::cell::Cell::new(false);
            {
                let _ = surgical_write(&file, &|current| {
                    let Ok(old) = std::str::from_utf8(current) else {
                        return Ok(None);
                    };
                    let mut doc: Yv = match serde_yaml::from_str(old) {
                        Ok(x) => x,
                        Err(err) => {
                            tracing::warn!(file=%file.display(), "DSH profile invalid; preserving it: {err}");
                            return Ok(None);
                        }
                    };
                    let Some(rows) = doc.as_sequence_mut() else {
                        return Ok(None);
                    };
                    let Some(row) = rows.iter_mut().find(|row| {
                        row.as_mapping()
                            .and_then(|m| m.get(&Yv::String("id".into())))
                            .and_then(Yv::as_str)
                            == Some("dsh-hooks")
                    }) else {
                        return Ok(None);
                    };
                    let Some(record) = row.as_mapping_mut() else {
                        return Ok(None);
                    };
                    if !record.contains_key(&Yv::String("config".into())) {
                        record.insert(Yv::String("config".into()), Yv::Mapping(serde_yaml::Mapping::new()));
                    }
                    let Some(config) = record
                        .get_mut(&Yv::String("config".into()))
                        .and_then(Yv::as_mapping_mut)
                    else {
                        return Ok(None);
                    };
                    if !config.contains_key(&Yv::String("hooks".into())) {
                        config.insert(Yv::String("hooks".into()), Yv::Sequence(Vec::new()));
                    }
                    let Some(hooks) = config
                        .get_mut(&Yv::String("hooks".into()))
                        .and_then(Yv::as_sequence_mut)
                    else {
                        return Ok(None);
                    };
                    if hooks.iter().any(|hook| {
                        hook.as_mapping()
                            .and_then(|m| m.get(&Yv::String("run".into())))
                            .and_then(Yv::as_str)
                            == Some(command.as_str())
                    }) {
                        return Ok(None);
                    }
                    let mut rule = serde_yaml::Mapping::new();
                    rule.insert(Yv::String("on".into()), Yv::String("turn/end".into()));
                    rule.insert(Yv::String("run".into()), Yv::String(command.clone().into()));
                    rule.insert(
                        Yv::String("timeoutMs".into()),
                        serde_yaml::to_value(5000)?,
                    );
                    hooks.push(Yv::Mapping(rule));
                    let new = serde_yaml::to_string(&doc)?;
                    if new == old {
                        return Ok(None);
                    }
                    touched.set(true);
                    Ok(Some(new.into_bytes()))
                })?;
            }
            if touched.get() {
                changed = true;
            }
        }
        Ok(if changed { InstallOutcome::Synced } else { InstallOutcome::Skipped })
    }

    #[cfg(unix)]
    fn verify(&self, home: &Path, shim: &Path) -> HookStatus {
        let root = root_of(home);
        let profiles = root.join("profiles");
        if !profiles.is_dir() {
            return HookStatus::Absent;
        }
        let command = run_command(shim);
        let mut found_profile = false;
        let mut installed = false;
        for entry in match std::fs::read_dir(&profiles) {
            Ok(d) => d,
            Err(_) => return HookStatus::Unreadable,
        }
        .flatten()
        .take(64)
        {
            if !entry.file_type().map(|x| x.is_dir()).unwrap_or(false) {
                continue;
            }
            let file = entry.path().join("cordis.patch.yml");
            let Ok(old) = std::fs::read_to_string(&file) else {
                continue;
            };
            let Ok(doc) = serde_yaml::from_str::<Yv>(&old) else {
                continue;
            };
            if let Some(rows) = doc.as_sequence() {
                if let Some(row) = rows.iter().find(|row| {
                    row.as_mapping()
                        .and_then(|m| m.get(&Yv::String("id".into())))
                        .and_then(Yv::as_str)
                        == Some("dsh-hooks")
                }) {
                    found_profile = true;
                    if let Some(hooks) = row
                        .get(&Yv::String("config".into()))
                        .and_then(|c| c.get(&Yv::String("hooks".into())))
                        .and_then(Yv::as_sequence)
                    {
                        installed = hooks.iter().any(|hook| {
                            hook.as_mapping()
                                .and_then(|m| m.get(&Yv::String("run".into())))
                                .and_then(Yv::as_str)
                                == Some(command.as_str())
                        });
                    }
                }
            }
        }
        if !found_profile {
            HookStatus::Absent
        } else if installed {
            HookStatus::Installed
        } else {
            HookStatus::Drifted
        }
    }
}
