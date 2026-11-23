//! Where something the person placed shows: in one thread, in every thread
//! of one stream, or in the whole project. Bookmarks use it (`bookmark`).
//! A `Viewer` — a thread and its stream — sees its thread's, its stream's
//! and the project's.
//!
//! Bare "scope" is not this: it is what a contribution may call on the
//! host (`.context/extensions.md`).

use oxplow_domain::{DomainError, StreamId, ThreadId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Where a placement shows: the thread it was made in, that thread's
/// stream, or the whole project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum VisibilityScope {
    Thread,
    Stream,
    Project,
}

impl VisibilityScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Thread => "thread",
            Self::Stream => "stream",
            Self::Project => "project",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "thread" => Some(Self::Thread),
            "stream" => Some(Self::Stream),
            "project" => Some(Self::Project),
            _ => None,
        }
    }
}

/// Who's looking: the thread (if any) and its stream (if any). Decides
/// what is visible, and owns a thread- or stream-scoped placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Viewer {
    pub thread: Option<ThreadId>,
    pub stream: Option<StreamId>,
}

impl Viewer {
    /// The `(thread_id, stream_id)` columns of a placement at `scope` for
    /// this viewer; `what` names it in the error ("bookmark").
    pub fn owner(
        self,
        scope: VisibilityScope,
        what: &str,
    ) -> Result<(Option<i64>, Option<i64>), DomainError> {
        match scope {
            VisibilityScope::Thread => self
                .thread
                .map(|t| (Some(t.value()), None))
                .ok_or_else(|| DomainError::Invalid(format!("a thread {what} needs a thread"))),
            VisibilityScope::Stream => self
                .stream
                .map(|s| (None, Some(s.value())))
                .ok_or_else(|| DomainError::Invalid(format!("a stream {what} needs a stream"))),
            VisibilityScope::Project => Ok((None, None)),
        }
    }
}
