//! OpenCode: global JS plugin at `~/.config/opencode/plugins/ema-report.js`.
//! The plugin reads session state through the client API on `session.idle`.
use super::normalize;
use crate::adapter::{AgentAdapter, HookStatus, InstallOutcome};
use crate::event::NormalizedEvent;
use crate::util::{is_managed_file, owned_file};
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

pub struct Opencode;

fn plugin_path(home: &Path) -> std::path::PathBuf {
    home.join(".config/opencode/plugins/ema-report.js")
}

fn plugin_source(shim: &Path) -> String {
    // EMA-MANAGED-HOOK: global OpenCode completion plugin.
    const TEMPLATE: &str = r###"// EMA-MANAGED-HOOK: global OpenCode completion plugin.
import { spawn } from "node:child_process";
const script = __SCRIPT__;
export const EmaReport = async ({ directory, client }) => ({
  event: async ({ event }) => {
    if (event.type !== "session.idle") return;
    const props = event.properties ?? event.data ?? {};
    const id = props.sessionID ?? props.sessionId ?? "";
    let title = "", prompt = "", response = "", contextMessages = [];
    if (id) {
      try {
        const [session, result] = await Promise.all([
          client.session.get({ path: { id } }), client.session.messages({ path: { id } })
        ]);
        title = session.data?.title ?? "";
        const messages = result.data ?? [];
        const text = m => (m?.parts ?? []).filter(p => p.type === "text").map(p => p.text).join("\n");
        const last = messages.findLastIndex(m => m.info?.role === "assistant" && text(m));
        if (last >= 0) {
          response = text(messages[last]);
          const before = messages.slice(0, last);
          const input = before.map(m => m.info?.role === "user").lastIndexOf(true);
          if (input >= 0) {
            prompt = text(before[input]);
            contextMessages = before.slice(0, input).filter(m => ["user", "assistant"].includes(m.info?.role) && text(m))
              .slice(-2).map(m => ({role: m.info.role, text: Array.from(text(m)).slice(0, 1001).join("")}));
          }
        }
      } catch (_) {}
    }
    const payload = JSON.stringify({
      type: "session.idle", session_id: id, session_title: title,
      event_id: event.id ?? event.created ?? Date.now(),
      cwd: directory ?? "", context_messages: contextMessages,
      prompt: prompt.slice(0, 4000), response: response.slice(0, 10000),
      prompt_truncated: prompt.length > 4000, response_truncated: response.length > 10000
    });
    try {
      const p = spawn(script, ["opencode"], { stdio: ["pipe", "ignore", "ignore"] });
      p.on("error", () => {});
      p.stdin.on("error", () => {});
      p.stdin.end(payload);
    } catch (_) {}
  }
});
"###;
    TEMPLATE.replace("__SCRIPT__", &serde_json::to_string(&shim.to_string_lossy().to_string()).unwrap())
}

impl AgentAdapter for Opencode {
    fn name(&self) -> &'static str {
        "opencode"
    }
    fn wire_events(&self) -> &[&'static str] {
        &["session.idle"]
    }
    fn normalize(&self, raw: &Value) -> Option<NormalizedEvent> {
        normalize("opencode", raw, self.wire_events())
    }

    #[cfg(unix)]
    fn install(&self, home: &Path, shim: &Path) -> Result<InstallOutcome> {
        let dir = home.join(".config/opencode");
        if !dir.is_dir() {
            return Ok(InstallOutcome::Skipped);
        }
        let wrote = owned_file(&plugin_path(home), &plugin_source(shim))?;
        Ok(if wrote { InstallOutcome::Synced } else { InstallOutcome::Skipped })
    }

    #[cfg(unix)]
    fn verify(&self, home: &Path, shim: &Path) -> HookStatus {
        if !home.join(".config/opencode").is_dir() {
            return HookStatus::Absent;
        }
        let file = plugin_path(home);
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
        let file = plugin_path(home);
        if is_managed_file(&file) {
            std::fs::remove_file(&file)?;
        }
        Ok(())
    }
}
