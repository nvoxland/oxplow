//! Pure domain types and store traits for oxplow.
//!
//! This crate is the foundation of the workspace — it defines the
//! data shapes (streams, threads, tasks, hook events) and the
//! abstract traits that infrastructure crates implement. It contains
//! no IO, no async runtime usage, and no platform-specific code.

pub mod agent;
pub mod capability;
pub mod code_intel;
pub mod commands;
pub mod comment;
pub mod error;
pub mod events;
pub mod hook;
pub mod host_capability;
pub mod ids;
pub mod json;
pub mod knowledge;
pub mod refs;
pub mod replaceable;
pub mod snapshot;
pub mod stores;
pub mod stream;
pub mod task;
pub mod thread;
pub mod time;
pub mod tree_diff;
pub mod vcs;
pub mod vocabulary;
pub mod work_items;

pub use agent::AgentKind;
pub use commands::{
    namespace_of, Actor, Atomicity, CommandCall, CommandEffect, CommandError, CommandOutcome,
    CommandSpec, CommandUi, Confirm, InputValidator, Invoker, Invokers, Lifecycle, Preview,
    OXPLOW_NAMESPACE,
};
pub use comment::{
    Comment, CommentIntent, CommentMessage, CommentStatus, CommentTarget, CommentThread,
};
pub use error::DomainError;
pub use events::schema::{EventSchemaRegistry, EventType};
pub use events::{Anchors, Envelope, EventId, StoredEvent};
pub use hook::{AgentStatus, AgentStatusState, AgentTurn, HookKind};
pub use ids::{
    AgentTurnId, AnyId, CommentId, CommentMessageId, DashboardId, DashboardItemId, EffortId,
    EntityKind, FollowupId, IdParseError, NoteId, PageVisitId, StreamId, TaskId, TaskLinkId,
    ThreadId, UsageEventId,
};
pub use json::Json;
pub use stream::{Stream, StreamKind};
pub use task::{
    Task, TaskActorKind, TaskAuthor, TaskImpact, TaskLink, TaskLinkType, TaskNote, TaskPriority,
    TaskStatus,
};
pub use thread::{Thread, ThreadNote, ThreadStatus};
pub use time::Timestamp;
pub use tree_diff::{diff_trees, ChangeStatus, FileChange};
