//! The chat machinery, kept as a library: a client of an agent's
//! structured protocol (ACP, or Codex's app-server), the transcript it
//! builds and the policy it applies to what the agent asks. No session tab
//! shows it — a session is the agent's own TUI in a terminal. Today it
//! runs headless: the Codex connection observes the thread a Codex session's
//! TUI is attached to. [`manager::ChatManager::open`], which spawns an ACP
//! program over its stdio, has no production caller until a later embedded
//! chat; its tests (`tests/acp_services.rs`, `tests/acp_live.rs`) and the
//! `oxplow-acp-fake` agent keep it working. See `.context/agent-model.md`
//! → "The chat machinery (library)" and "Codex: its TUI on a thread oxplow
//! observes".
//!
//! `conn` is what the session actor needs of either connection; `acp` (the
//! only module that names ACP SDK types) and `codex` are the two. `model`
//! is oxplow's side of what they say, `mapping` turns tool calls into policy
//! intents and canonical events, `transcript` holds the conversation.

pub mod acp;
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
