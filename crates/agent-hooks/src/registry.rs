//! The built-in adapter registry. Single source of truth for:
//! * which agents exist (drives the shim's dispatch list and spool allow-list),
//! * how each agent's hook is installed and verified,
//! * how each agent's wire payload becomes a canonical event.
use std::path::Path;
use std::sync::Arc;
use crate::adapters::all;
use crate::adapter::{AgentAdapter, HookStatus};

fn built_in() -> Vec<Arc<dyn AgentAdapter>> {
    all().into_iter().map(Arc::from).collect()
}

/// All built-in adapter names, in registry (shim dispatch) order.
pub fn agent_names() -> Vec<&'static str> {
    let adapters = built_in();
    adapters.iter().map(|a| a.name()).collect()
}

/// Find the adapter registered under `name`.
pub fn find(name: &str) -> Option<Arc<dyn AgentAdapter>> {
    built_in().into_iter().find(|a| a.name() == name)
}

/// Idempotently install/repair every adapter's hook. Per-adapter failures are
/// logged and never abort the rest of the sync.
#[cfg(unix)]
pub fn sync(home: &Path, shim: &Path) {
    for adapter in built_in() {
        if let Err(err) = adapter.install(home, shim) {
            tracing::warn!(agent=adapter.name(), "hook sync skipped: {err:#}");
        }
    }
}

/// Cheap read-only health pass over every adapter.
#[cfg(unix)]
pub fn verify_all(home: &Path, shim: &Path) -> Vec<(&'static str, HookStatus)> {
    built_in().into_iter().map(|a| (a.name(), a.verify(home, shim))).collect()
}

