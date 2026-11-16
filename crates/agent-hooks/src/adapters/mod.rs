//! One module per agent CLI. Each provides an [`AgentAdapter`] implementor.
use crate::{
    adapter::{AgentAdapter, wire_event_type},
    event::{NormalizedEvent, extract},
};

mod alma;
mod claude;
mod codex;
mod dsh;
mod gemini;
mod hermes;
mod kimi;
mod omp;
mod openclaw;
mod opencode;
mod pi;
mod zcode;

pub use alma::Alma;
pub use claude::Claude;
pub use codex::Codex;
pub use dsh::Dsh;
pub use gemini::Gemini;
pub use hermes::Hermes;
pub use kimi::Kimi;
pub use omp::Omp;
pub use openclaw::Openclaw;
pub use opencode::Opencode;
pub use pi::Pi;
pub use zcode::Zcode;

/// Build the canonical event when `raw` carries one of `allowed` wire types.
/// Every plugin-based and raw-payload adapter shares this extractor; agents
/// with a custom envelope (Alma) implement [`AgentAdapter::normalize`] directly.
pub(crate) fn normalize(agent: &'static str, raw: &Value, allowed: &[&str]) -> Option<NormalizedEvent> {
    let typ = wire_event_type(raw);
    if !allowed.iter().any(|e| *e == typ) {
        return None;
    }
    Some(extract(agent, typ, raw))
}

pub(crate) use serde_json::Value;

/// Collect all built-in adapters in registry order. The order defines the
/// spool shim's dispatch list, so keep it stable.
#[allow(clippy::type_complexity)]
pub(crate) fn all() -> Vec<Box<dyn AgentAdapter>> {
    vec![
        Box::new(Codex),
        Box::new(Claude),
        Box::new(Gemini),
        Box::new(Opencode),
        Box::new(Pi),
        Box::new(Omp),
        Box::new(Kimi),
        Box::new(Zcode),
        Box::new(Hermes),
        Box::new(Openclaw),
        Box::new(Dsh),
        Box::new(Alma),
    ]
}
