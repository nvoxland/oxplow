//! Agents spoken to over the Agent Client Protocol (tsk281): which ACP
//! agents a project can run, and the client that runs them. See
//! `.context/agent-model.md` → "ACP agents".
//!
//! `wire` is the only module that names SDK types; `model` is oxplow's
//! side of them, `mapping` turns tool calls into policy intents and
//! canonical events, `transcript` holds the conversation.

pub mod agents;
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
