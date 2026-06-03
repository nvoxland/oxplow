//! Comments: threaded annotations anchored to a text selection on any
//! page (wiki body, code file lines, task detail, …).
//!
//! A [`Comment`] is the thread anchor + metadata; the back-and-forth
//! lives in [`CommentMessage`] rows (including the first message). The
//! pair travels together as a [`CommentThread`] so callers fetch a
//! whole conversation in one shot.
//!
//! Anchoring is deliberately resilient rather than positional: `quote`
//! (the selected text) is the durable anchor and the context handed to
//! the agent; `selectors_json` is a W3C-Web-Annotation selectors array
//! (TextQuoteSelector + TextPositionSelector + an optional per-surface
//! coordinate selector) that the renderer re-validates on load and may
//! rewrite. When the quote can no longer be located the comment is
//! marked `orphaned` — it still shows in the inbox, just without an
//! inline highlight.
//!
//! Beyond the quote, a comment carries the *typed* context it was made
//! in: `context_chain` is the nesting of page regions the selection sat
//! inside (innermost→outermost, e.g. a file row under a commit), and
//! `referenced_refs` are the canonical refs found inside the selection
//! itself (links + inline mentions). Both reuse the [`CommentTarget`]
//! `(kind,id)` shape — the same canonical `page_ref` vocabulary — so the
//! agent can hydrate them into typed summaries when answering.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ids::{CommentId, CommentMessageId, StreamId, ThreadId};
use crate::time::Timestamp;

/// Why the comment exists — drives what the agent acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CommentIntent {
    /// A private thinking note; the agent leaves it alone unless asked.
    Note,
    /// The user wants the agent to do something about this.
    Followup,
}

/// Lifecycle of a comment thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum CommentStatus {
    Open,
    Resolved,
}

/// A canonical cross-page reference: `kind` is the page-kind scheme
/// (`"wiki" | "file" | "directory" | "task" | "git-commit" | "finding"`,
/// extensible) and `id` is the canonical id for that kind — wiki slug,
/// worktree-relative path, task id as a string, commit sha, etc. This is
/// the same `(kind,id)` vocabulary the `page_ref` graph and tab ids use.
///
/// Used as a comment's primary anchor target and, in `Vec` form, as the
/// `context_chain` (ancestor regions) and `referenced_refs` (refs inside
/// the selection).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct CommentTarget {
    pub kind: String,
    pub id: String,
}

/// The thread anchor + metadata. The conversation lives in
/// [`CommentMessage`] rows keyed by `id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Comment {
    pub id: CommentId,
    /// Hard scope — comments are queryable per stream regardless of
    /// which thread authored them.
    pub stream_id: StreamId,
    /// The agent thread the comment was first added in. Nullable so a
    /// content comment survives the thread being archived.
    pub thread_id: Option<ThreadId>,
    pub target_kind: String,
    pub target_id: String,
    pub quote: String,
    /// W3C selectors array (opaque to the store; the renderer parses it).
    pub selectors_json: String,
    /// Ancestor regions the selection sat inside, innermost→outermost,
    /// EXCLUDING the primary target. Empty for a top-level selection.
    pub context_chain: Vec<CommentTarget>,
    /// Canonical refs found inside the selection (links + mentions).
    pub referenced_refs: Vec<CommentTarget>,
    pub intent: CommentIntent,
    pub status: CommentStatus,
    pub orphaned: bool,
    pub author: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub last_activity_at: Timestamp,
    /// When the comment was last moved to `resolved`, or `None` while
    /// open. Cleared on reopen. Distinct from `updated_at` (which auto
    /// re-anchoring bumps) and `last_activity_at` (messages only), so it
    /// is the only reliable "when resolved" signal.
    pub resolved_at: Option<Timestamp>,
}

/// One message in a comment thread (the first message included).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct CommentMessage {
    pub id: CommentMessageId,
    pub comment_id: CommentId,
    /// Free-form, e.g. `"user"` or `"agent"`.
    pub author: String,
    pub body: String,
    pub created_at: Timestamp,
}

/// A comment plus its full message thread, oldest-first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct CommentThread {
    pub comment: Comment,
    pub messages: Vec<CommentMessage>,
}

impl CommentThread {
    /// True when this is an open follow-up whose latest message is not
    /// from the agent — i.e. the user said something the agent hasn't
    /// answered yet. "Answered" is derived from the conversation tail
    /// (messages are stored oldest-first) rather than a stored flag, so
    /// a user reply after an agent response re-opens the follow-up.
    pub fn needs_response(&self) -> bool {
        if self.comment.intent != CommentIntent::Followup
            || self.comment.status != CommentStatus::Open
        {
            return false;
        }
        match self.messages.last() {
            Some(last) => last.author != "agent",
            None => false,
        }
    }
}
