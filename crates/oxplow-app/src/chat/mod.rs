//! Observing an agent through its structured protocol: a Codex session is
//! Codex's own TUI in a terminal, on a thread of Codex's app-server daemon
//! that oxplow opens and watches. The session actor (`session`) records
//! what the agent does, the way a hooked agent's turns are recorded; it
//! sends nothing but context between turns and a rename. See
//! `.context/agent-model.md` → "Codex: its TUI on a thread oxplow observes".
//!
//! `conn` is what the actor needs of a connection; `codex` is the one.
//! `model` is oxplow's side of what a connection says, `mapping` turns tool
//! calls into what is recorded.

pub mod codex;
pub mod conn;
pub mod host;
pub mod manager;
pub mod mapping;
pub mod model;
pub mod session;

#[cfg(test)]
mod guard_tests;
#[cfg(test)]
mod session_tests;
