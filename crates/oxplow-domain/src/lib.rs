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
pub mod capability;
pub mod code_intel;
pub mod commands;
pub mod comment;
pub mod contribution;
pub mod diagnostic;
pub mod effort_impact;
pub mod effort_policy;
pub mod error;
pub mod events;
pub mod extension_job;
pub mod extension_job_context;
pub mod extension_path;
pub mod extension_secret;
pub mod extension_state;
pub mod filesystem;
pub mod grant;
pub mod hook;
pub mod ids;
pub mod json;
pub mod knowledge;
pub mod language_id;
pub mod ordering;
pub mod permissions;
pub mod replaceable;
pub mod row_ids;
pub mod run;
pub mod scope;
pub mod service;
pub mod snapshot;
pub mod stores;
pub mod stream;
pub mod template;
pub mod thread;
pub mod time;
pub mod tree_diff;
pub mod uri;
pub mod vcs;
pub mod vocabulary;
pub mod when;
pub mod work_items;

pub use ai_context::AiContext;
pub use commands::{
    namespace_of, Access, Actor, Atomicity, CommandCall, CommandError, CommandOutcome, CommandSpec,
    CommandUi, Confirm, InputValidator, Invoker, Lifecycle, MenuPlace, OpRef, Preview,
    OXPLOW_NAMESPACE,
};
pub use comment::{Comment, CommentIntent, CommentMessage, CommentStatus, CommentThread};
pub use effort_impact::EffortImpact;
pub use error::DomainError;
pub use events::schema::{EventSchemaRegistry, EventType};
pub use events::{Anchors, Envelope, EventId, StoredEvent};
pub use hook::{AgentStatus, AgentStatusState, AgentTurn, HookKind};
pub use ids::{
    AgentSessionId, AgentTurnId, AnyId, CommentId, CommentMessageId, EffortId, EntityKind,
    HandoffId, IdParseError, NoteId, PageVisitId, StreamId, TerminalId, ThreadId, UsageEventId,
};
pub use json::{JsNumber, Json};
pub use row_ids::{
    AiCallId, AuditId, ByteSize, DeadLetterId, FileSnapshotId, ProposalId, SnapshotId,
};
pub use stream::{HostId, Stream, StreamKind};
pub use thread::{Thread, ThreadMark};
pub use time::Timestamp;
pub use tree_diff::{diff_trees, ChangeStatus, FileChange};
