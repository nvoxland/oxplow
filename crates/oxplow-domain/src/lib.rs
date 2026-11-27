//! Pure domain types and store traits for oxplow.
//!
//! This crate is the foundation of the workspace — it defines the
//! data shapes (streams, threads, tasks, hook events) and the
//! abstract traits that infrastructure crates implement. It contains
//! no IO, no async runtime usage, and no platform-specific code.

pub mod agent;
pub mod agent_session;
pub mod ai;
pub mod ai_context;
pub mod built;
pub mod capability;
pub mod client;
pub mod code_intel;
pub mod commands;
pub mod contribution;
pub mod diagnostic;
pub mod error;
pub mod events;
pub mod extension_path;
pub mod extension_secret;
pub mod extension_state;
pub mod filesystem;
pub mod folder_root;
pub mod grant;
pub mod hook;
pub mod ids;
pub mod job_run;
pub mod job_run_context;
pub mod json;
pub mod language_id;
pub mod ordering;
pub mod permissions;
pub mod replaceable;
pub mod row_ids;
pub mod run;
pub mod scope;
pub mod service;
pub mod settings;
pub mod snapshot;
pub mod stores;
pub mod stream;
pub mod tasks;
pub mod template;
pub mod thread;
pub mod time;
pub mod tree_diff;
pub mod uri;
pub mod vcs;
pub mod vocabulary;
pub mod when;

pub use ai_context::AiContext;
pub use client::{Caller, ConnectionId, DeviceDescriptor, DeviceId, DeviceKind};
pub use commands::{
    namespace_of, Access, Actor, Atomicity, CommandCall, CommandError, CommandOutcome, CommandSpec,
    CommandUi, Confirm, InputValidator, Invoker, Lifecycle, MenuPlace, OpRef, Preview,
    OXPLOW_NAMESPACE,
};
pub use error::DomainError;
pub use events::schema::{EventSchemaRegistry, EventType};
pub use events::{Anchors, Envelope, EventId, StoredEvent};
pub use hook::{AgentStatus, AgentStatusState, AgentTurn, HookKind};
pub use ids::{
    AgentSessionId, AgentTurnId, AnyId, EntityKind, IdParseError, NoteId, PageVisitId, RequestId,
    StreamId, TerminalId, ThreadId, UsageEventId,
};
pub use json::{JsNumber, Json};
pub use row_ids::{
    AiCallId, ByteSize, CommandRunId, DeadLetterId, FileSnapshotId, ProposalId, SnapshotId,
};
pub use stream::{HostId, Stream, StreamKind};
pub use thread::{Thread, ThreadMark};
pub use time::Timestamp;
pub use tree_diff::{diff_trees, ChangeStatus, FileChange};
