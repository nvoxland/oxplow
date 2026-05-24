//! Tauri command surface — split by area.
//!
//! Adding a new command: drop it in the relevant submodule, then
//! add the function name to `specta_builder()`'s `collect_commands![]`
//! list in `lib.rs`. The TS bindings regenerate on the next
//! `cargo test`.

pub mod agent_panes;
pub mod app;
pub mod background;
pub mod backlog;
pub mod branch;
pub mod churn;
pub mod code_quality;
pub mod comments;
pub mod config;
pub mod effort;
pub mod followup;
pub mod git;
pub mod hooks;
pub mod launch;
pub mod log;
pub mod lsp;
pub mod menu;
pub mod notes;
pub mod page_refs;
pub mod page_visit;
pub mod search;
pub mod snapshot;
pub mod streams;
pub mod tasks;
pub mod terminal;
pub mod threads;
pub mod usage;
pub mod webview;
pub mod wiki;
pub mod wiki_freshness;
pub mod workspace;
