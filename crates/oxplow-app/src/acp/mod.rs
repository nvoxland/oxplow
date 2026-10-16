//! Chat sessions: agents oxplow speaks to as a client and shows in its
//! own transcript — ACP agents (which ones a project can run, `agents`)
//! and threads on Codex's app-server daemon. See `.context/agent-model.md`
//! → "ACP agents" and "Codex: an app-server client".
//!
//! `conn` is what the session actor needs of either; `wire` (the only
//! module that names ACP SDK types) and `codex` are the two connections.
//! `model` is oxplow's side of what they say, `mapping` turns tool calls
//! into policy intents and canonical events, `transcript` holds the
//! conversation.

pub mod agents;
pub mod codex;
pub mod conn;
pub mod host;
pub mod human_prompt;
pub mod manager;
pub mod mapping;
pub mod model;
pub mod session;
pub mod transcript;
pub mod wire;

#[cfg(test)]
mod guard_tests;
#[cfg(test)]
mod session_tests;
