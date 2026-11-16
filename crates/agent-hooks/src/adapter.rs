//! The adapter contract. One implementor per agent CLI.
use std::path::Path;
use serde_json::Value;
use anyhow::Result;
use crate::event::NormalizedEvent;

/// What a verify pass found for one agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookStatus {
    /// The agent is not installed on this machine (no profile dir). Nothing to do.
    Absent,
    /// The EMA hook is present and active.
    Installed,
    /// The agent is installed but the EMA hook is missing or no longer points
    /// at the managed shim; a re-sync should repair it.
    Drifted,
    /// The owner explicitly disabled the EMA hook; must never be re-enabled.
    Denied,
    /// The hook slot is owned by someone else (e.g. a user-set Codex notify);
    /// EMA must not take it over.
    Foreign,
    /// The agent config exists but is unreadable/incompatible; EMA left it alone.
    Unreadable,
}

/// Outcome of an install attempt, for observability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallOutcome {
    /// Agent absent or nothing to change; no files were written.
    Skipped,
    /// Files were written or the existing managed hook was re-asserted.
    Synced,
}

/// A single agent's hook integration: installation, health checks, and
/// wire-payload normalization. Implementations must be:
/// * idempotent — running install twice changes nothing after the first pass;
/// * conservative — user-owned config content, invalid configs and explicit
///   disables are preserved and reported, never overwritten;
/// * credential-free — hooks only ever hand payloads to the local spool shim.
pub trait AgentAdapter: Send + Sync {
    /// Registry name; matches the spool file prefix and the shim dispatch arg.
    fn name(&self) -> &'static str;

    /// Native wire event types this adapter accepts. Anything else is dropped
    /// during normalization.
    fn wire_events(&self) -> &[&'static str];

    /// Normalize one raw wire payload (as spooled by the shim) into the
    /// canonical event, or `None` when the payload is not a reportable event.
    fn normalize(&self, raw: &Value) -> Option<NormalizedEvent>;

    /// Idempotently install/repair the EMA hook for this agent.
    #[cfg(unix)]
    fn install(&self, home: &Path, shim: &Path) -> Result<InstallOutcome>;

    /// Check the current state of the EMA hook (cheap config reads only).
    #[cfg(unix)]
    fn verify(&self, home: &Path, shim: &Path) -> HookStatus;

    /// Remove only the EMA-managed hook; user content and other hooks survive.
    #[cfg(unix)]
    fn uninstall(&self, home: &Path, shim: &Path) -> Result<()> {
        let _ = (home, shim);
        Ok(())
    }
}

/// Read the wire event type of a payload, from either common spelling.
pub fn wire_event_type(raw: &Value) -> &str {
    raw.get("type")
        .and_then(Value::as_str)
        .or_else(|| raw.get("hook_event_name").and_then(Value::as_str))
        .unwrap_or("")
}
