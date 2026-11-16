//! agent-hooks — unified local completion-hook adapters for coding agents.
//!
//! Each agent CLI exposes completion events through its own bespoke
//! mechanism (TOML notify slots, JSON settings hooks, JS/TS/Python plugins,
//! YAML profile patches, env-driven node scripts). This crate absorbs that
//! heterogeneity behind one contract:
//!
//! * [`adapter::AgentAdapter`] — per-agent install / verify / uninstall /
//!   normalize, with conservative, idempotent, user-preserving config writes;
//! * [`event::NormalizedEvent`] — the single canonical event schema;
//! * [`registry`] — the single source of truth for which agents exist,
//!   driving the spool shim's dispatch list and the drain allow-list;
//! * [`shim`] — the local spool executable that agent hooks pipe into.
//!
//! No hook ever sees SMTP credentials or touches the network: payloads land
//! in a private spool directory owned by the host application.
pub mod adapter;
pub mod event;
pub mod registry;
pub mod shim;
pub mod util;
pub mod adapters;
