//! Chat sessions: agents oxplow speaks to as a client and shows in its
//! own transcript — ACP agents (`acp`) and threads on Codex's app-server
//! daemon. See `.context/agent-model.md` → "Chat sessions" and "Codex: an
//! app-server client".
//!
//! `conn` is what the session actor needs of either connection; `acp::wire`
//! (the only module that names ACP SDK types) and `codex` are the two.
//! `model` is oxplow's side of what they say, `mapping` turns tool calls
//! into policy intents and canonical events, `transcript` holds the
//! conversation.

pub mod codex;
pub mod conn;
pub mod host;
pub mod human_prompt;
pub mod manager;
pub mod mapping;
pub mod model;
pub mod session;
pub mod transcript;

#[cfg(test)]
mod guard_tests;
#[cfg(test)]
mod session_tests;
