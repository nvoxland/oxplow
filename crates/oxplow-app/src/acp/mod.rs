//! Agents spoken to over the Agent Client Protocol (tsk281): which ACP
//! agents a project can run, and the client that runs them. See
//! `.context/agent-model.md` → "ACP agents".
//!
//! `wire` is the only module that names SDK types; `model` is oxplow's
//! side of them, `mapping` turns tool calls into policy intents and
//! canonical events, `transcript` holds the conversation.

pub mod agents;
pub mod mapping;
pub mod model;
pub mod transcript;
pub mod wire;
