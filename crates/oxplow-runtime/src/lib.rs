//! Runtime services: the write guard. Pure logic on top of store
//! traits — no IO, no Tauri awareness.
//!
//! This crate is callable from both `oxplow-tauri-ipc` (when a
//! command needs to render a guard decision into an HTTP-ish reply)
//! and from `oxplow-mcp` (when an MCP tool needs to honor the same
//! rules). Do not put DB calls, file IO, or HTTP here — wrap those at
//! the `oxplow-app` layer.

pub mod policy;
pub mod write_guard;

pub use policy::{
    decide_tool, path_outside_worktree, DenyLayer, IntentKind, PolicyDecision, PolicyFacts,
    ToolIntent,
};
pub use write_guard::{write_guard_reason, WriteGuardContext, WORKTREE_MUTATING_TOOLS};
