//! Runtime services: write guard, filing enforcement, and the
//! agent-turn lifecycle hook surface. Pure logic on top of store
//! traits — no IO, no Tauri awareness.
//!
//! This crate is callable from both `oxplow-tauri-ipc` (when a
//! command needs to render a guard decision into an HTTP-ish reply)
//! and from `oxplow-mcp` (when an MCP tool needs to honor the same
//! rules). Do not put DB calls, file IO, or HTTP here — wrap those at
//! the `oxplow-app` layer.

pub mod filing;
pub mod policy;
pub mod stop_hook;
pub mod write_guard;

pub use filing::{
    build_filing_enforcement_pre_tool_deny, build_filing_enforcement_pre_tool_reason,
    is_plan_mode_plan_file, FilingEnforcementContext, FilingEnforcementDeny,
    ALWAYS_WRITE_INTENT_TOOL_NAMES,
};
pub use policy::{
    decide_tool, path_outside_worktree, DenyLayer, IntentKind, PolicyDecision, PolicyFacts,
    ToolIntent,
};
pub use stop_hook::{
    compute_audit_signature, decide_stop_directive, find_stale_epic_children_pairs,
    DirectiveBuilders, OpenEffort, StaleEpicPair, StopDirective, StopHookOutcome,
    StopHookSideEffect, ThreadSnapshot,
};
pub use write_guard::{
    build_write_guard_response, WriteGuardContext, WriteGuardDeny, WORKTREE_MUTATING_TOOLS,
};
