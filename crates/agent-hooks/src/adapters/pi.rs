//! Pi: global TS extension at `~/.pi/agent/extensions/ema-report.ts`.
//!
//! Pi's `agent_end` fires per low-level run and can precede an auto-retry,
//! compaction retry or queued continuation. `agent_settled` fires exactly
//! once when the run will not continue automatically, so it is the correct
//! single completion signal for reporting. It carries no messages, so the
//! final assistant text is read back from the read-only session manager.
use super::normalize;
use crate::adapter::{AgentAdapter, HookStatus, InstallOutcome};
use crate::event::NormalizedEvent;
use crate::util::{is_managed_file, owned_file};
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

pub struct Pi;

fn extension_path(home: &Path) -> std::path::PathBuf {
    home.join(".pi/agent/extensions/ema-report.ts")
}

fn extension_source(shim: &Path) -> String {
    const TEMPLATE: &str = r###"// EMA-MANAGED-HOOK: global Pi extension. Reports once per settled run.
import { spawn } from "node:child_process";
const script = __SCRIPT__;
export default function (pi) {
  pi.on("agent_settled", async (event, ctx) => {
    if (event.aborted) return;
    const entries = (ctx.sessionManager?.getBranch?.() ?? ctx.sessionManager?.getEntries?.()) || [];
    let response = "", responseId = "", prompt = "", promptIndex = -1;
    const asText = c => Array.isArray(c) ? c.filter(x => x?.type === "text").map(x => x.text).join("\n")
      : (typeof c === "string" ? c : "");
    for (let i = entries.length - 1; i >= 0; i--) {
      const m = entries[i]?.type === "message" ? entries[i].message : null;
      if (m?.role !== "user") continue;
      const c = m.content;
      prompt = Array.isArray(c) ? c.filter(x => x?.type === "text").map(x => x.text).join("\n")
      : (typeof c === "string" ? c : "");
      if (prompt) { promptIndex = i; break; }
    }
    const contextMessages = entries.slice(0, Math.max(promptIndex, 0))
      .filter(e => e?.type === "message" && ["user", "assistant"].includes(e.message?.role) && asText(e.message.content))
      .slice(-2).map(e => ({role: e.message.role, text: Array.from(asText(e.message.content)).slice(0, 1001).join("")}));
    for (let i = entries.length - 1; i >= 0; i--) {
      const entry = entries[i];
      const m = entry && entry.type === "message" ? entry.message : null;
      if (!m || m.role !== "assistant") continue;
      const c = m.content;
      const text = Array.isArray(c)
        ? c.filter(x => x && x.type === "text").map(x => x.text).join("\n")
        : (typeof c === "string" ? c : "");
      if (text) { response = text; responseId = m.responseId || ""; break; }
    }
    const payload = JSON.stringify({
      type: "agent_end",
      session_id: (ctx.sessionManager && ctx.sessionManager.getSessionFile
        ? ctx.sessionManager.getSessionFile() : null) || "",
      event_id: responseId,
      cwd: ctx.cwd || "",
      session_title: pi.getSessionName?.() || "",
      context_messages: contextMessages,
      prompt: prompt.slice(0, 4000),
      response: response.slice(0, 10000),
      prompt_truncated: prompt.length > 4000, response_truncated: response.length > 10000
    });
    try {
      const p = spawn(script, ["pi"], { stdio: ["pipe", "ignore", "ignore"] });
      p.on("error", () => {});
      p.stdin.on("error", () => {});
      p.stdin.end(payload);
    } catch (_) {}
  });
}
"###;
    TEMPLATE.replace("__SCRIPT__", &serde_json::to_string(&shim.to_string_lossy().to_string()).unwrap())
}

impl AgentAdapter for Pi {
    fn name(&self) -> &'static str {
        "pi"
    }
    fn wire_events(&self) -> &[&'static str] {
        &["agent_end"]
    }
    fn normalize(&self, raw: &Value) -> Option<NormalizedEvent> {
        normalize("pi", raw, self.wire_events())
    }

    #[cfg(unix)]
    fn install(&self, home: &Path, shim: &Path) -> Result<InstallOutcome> {
        let dir = home.join(".pi/agent");
        if !dir.is_dir() {
            return Ok(InstallOutcome::Skipped);
        }
        let wrote = owned_file(&extension_path(home), &extension_source(shim))?;
        Ok(if wrote { InstallOutcome::Synced } else { InstallOutcome::Skipped })
    }

    #[cfg(unix)]
    fn verify(&self, home: &Path, shim: &Path) -> HookStatus {
        if !home.join(".pi/agent").is_dir() {
            return HookStatus::Absent;
        }
        let file = extension_path(home);
        let Ok(source) = std::fs::read_to_string(&file) else {
            return HookStatus::Drifted;
        };
        if !source.contains(crate::util::MARKER) {
            return HookStatus::Foreign;
        }
        if source.contains(&serde_json::to_string(&shim.to_string_lossy().to_string()).unwrap()) {
            HookStatus::Installed
        } else {
            HookStatus::Drifted
        }
    }

    #[cfg(unix)]
    fn uninstall(&self, home: &Path, _shim: &Path) -> Result<()> {
        let file = extension_path(home);
        if is_managed_file(&file) {
            std::fs::remove_file(&file)?;
        }
        Ok(())
    }
}
