//! Command "core" functions: the real request-handling bodies, lifted
//! out of the Tauri `#[tauri::command]` wrappers so they can be called
//! both locally (oxplow-tauri-ipc) and remotely (oxplow-daemon HTTP).
//!
//! Convention: every core takes `svc: &Services` as its first parameter
//! (even when unused) so the dispatch registry in `crate::lib` can call
//! them uniformly. Remaining parameters mirror the original command's
//! args one-for-one, in declaration order. One file per command module,
//! mirroring `oxplow-tauri-ipc/src/commands/`.
//!
//! Request/response structs that used to live next to the Tauri
//! adapters move here with their `derive(Serialize, Deserialize, Type)`
//! intact; the tauri-ipc module re-exports them so the specta TS export
//! is unchanged.
//!
//! NOT here (Tauri-only, never dispatched): `launch`, `menu`, `webview`
//! — they touch the OS window/clipboard and stay in the shell.

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
pub mod log;
pub mod lsp;
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
pub mod wiki;
pub mod wiki_freshness;
pub mod workspace;
