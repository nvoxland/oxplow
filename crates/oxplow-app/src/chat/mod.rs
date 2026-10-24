//! The chat machinery, kept as a library: the session actor (`session`), a
//! client of an agent's structured protocol behind the `conn::Connection`
//! trait, the transcript it builds and the policy it applies to what the
//! agent asks. No session tab shows it — a session is the agent's own TUI
//! in a terminal. Today it runs headless: the Codex connection observes the
//! thread a Codex session's TUI is attached to. Nothing speaks ACP. See
//! `.context/agent-model.md` → "The chat machinery (a library)" and
//! "Codex: its TUI on a thread oxplow observes".
//!
//! `conn` is what the session actor needs of a connection; `codex` is the
//! one. `model` is oxplow's side of what a connection says, `mapping` turns
//! tool calls into policy intents and canonical events, `transcript` holds
//! the conversation.

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
