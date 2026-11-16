//! OMP: user-level `agent_end` observer at `~/.omp/agent/hooks/post/ema-report.ts`.
//! OMP loads user-scope factories from `~/.omp/agent/hooks/{pre,post}/*.ts`.
use super::normalize;
use crate::adapter::{AgentAdapter, HookStatus, InstallOutcome};
use crate::event::NormalizedEvent;
use crate::util::{is_managed_file, owned_file};
use anyhow::Result;
use serde_json::Value;
use std::path::Path;

pub struct Omp;

fn hook_path(home: &Path) -> std::path::PathBuf {
    home.join(".omp/agent/hooks/post/ema-report.ts")
}

fn hook_source(shim: &Path) -> String {
    const TEMPLATE: &str = r###"// EMA-MANAGED-HOOK: OMP user-level agent_end observer.
import { spawn } from "node:child_process";
const script = __SCRIPT__;
export default function(pi) {
  pi.on("agent_end", async (event, ctx) => {
    if (event.willContinue || event.awaitingAsyncWork) return;
    const messages = event.messages ?? [];
    const last = messages.filter(m => m.role === "assistant").at(-1);
    const input = messages.filter(m => m.role === "user").at(-1);
    const asText = c => typeof c === "string" ? c : (Array.isArray(c) ? c : []).filter(x => x?.type === "text").map(x => x.text).join("\n");
    const response = asText(last?.content);
    const prompt = asText(input?.content);
    const payload = JSON.stringify({
      type: "agent_end", event_id: Date.now(),
      session_id: ctx.sessionManager?.getSessionFile?.() ?? "",
      cwd: ctx.cwd ?? "", prompt: prompt.slice(0, 4000), response: response.slice(0, 10000),
      prompt_truncated: prompt.length > 4000, response_truncated: response.length > 10000
    });
    try {
      const p = spawn(script, ["omp"], { stdio: ["pipe","ignore","ignore"] });
      p.on("error", () => {});
      p.stdin.on("error", () => {});
      p.stdin.end(payload);
    } catch (_) {}
  });
}
"###;
    TEMPLATE.replace("__SCRIPT__", &serde_json::to_string(&shim.to_string_lossy().to_string()).unwrap())
}

impl AgentAdapter for Omp {
    fn name(&self) -> &'static str {
        "omp"
    }
    fn wire_events(&self) -> &[&'static str] {
        &["agent_end"]
    }
    fn normalize(&self, raw: &Value) -> Option<NormalizedEvent> {
        normalize("omp", raw, self.wire_events())
    }

    #[cfg(unix)]
    fn install(&self, home: &Path, shim: &Path) -> Result<InstallOutcome> {
        let dir = home.join(".omp/agent");
        if !dir.is_dir() {
            return Ok(InstallOutcome::Skipped);
        }
        let wrote = owned_file(&hook_path(home), &hook_source(shim))?;
        Ok(if wrote { InstallOutcome::Synced } else { InstallOutcome::Skipped })
    }

    #[cfg(unix)]
    fn verify(&self, home: &Path, shim: &Path) -> HookStatus {
        if !home.join(".omp/agent").is_dir() {
            return HookStatus::Absent;
        }
        let file = hook_path(home);
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
        let file = hook_path(home);
        if is_managed_file(&file) {
            std::fs::remove_file(&file)?;
        }
        Ok(())
    }
}
