//! Cross-store event bus.
//!
//! Stores and services post `OxplowEvent` values onto a single
//! `tokio::sync::broadcast` channel. The Tauri layer subscribes once
//! and forwards each event to the renderer via `app_handle.emit`. The
//! MCP layer can subscribe independently if it ever needs to surface
//! state changes to the agent.
//!
//! Events are intentionally coarse: the renderer treats them as
//! "something in this bucket changed, refetch" rather than diffs.
//! The flat enum keeps the wire format simple and avoids a
//! per-bucket subscribe API.

use serde::{Deserialize, Serialize};
use specta::Type;
use tokio::sync::broadcast;

use oxplow_domain::{AgentStatusState, StreamId, TaskId, ThreadId};

/// Event channel names shared by every transport that carries backend
/// events to the renderer. The Tauri shell `app.emit`s on the channel
/// names; the daemon's `/events` WebSocket multiplexes them with the
/// frame keys; `apps/desktop/src/tauri-bridge/channels.ts` mirrors the
/// mapping for the renderer (pinned by the surface-parity test —
/// change either side and that test points at the other).
pub mod event_channels {
    /// `OxplowEvent` payloads (the cross-store bus).
    pub const OXPLOW: &str = "oxplow:event";
    /// LSP bridge events.
    pub const LSP: &str = "lsp:event";
    /// Terminal bridge events.
    pub const TERMINAL: &str = "terminal:event";

    /// Frame keys used as `{"channel": <key>, "payload": …}` on the
    /// daemon's multiplexed `/events` socket, keyed to the channel
    /// each frame demuxes back onto.
    pub const FRAMES: &[(&str, &str)] = &[("oxplow", OXPLOW), ("lsp", LSP), ("terminal", TERMINAL)];
}

/// fs-watch classification mirrored onto the wire so the renderer can
/// distinguish create / modify / delete / rename without re-stating
/// every variant of the upstream `notify` crate.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceChangeKind {
    Created,
    Updated,
    Deleted,
    Renamed,
}

/// Snapshot trigger source. The renderer renders these differently in
/// the Snapshots panel ("startup" rows are dimmer than "effort-end").
///
/// Tasks themselves don't have a start/end — only efforts do. The
/// Effort* variants are the snapshot bracket for a single effort
/// row's lifetime.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "kebab-case")]
pub enum SnapshotSourceKind {
    EffortStart,
    EffortEnd,
    EffortEvent,
    Startup,
    Manual,
    /// Triggered by a HEAD/refs change (commit, branch switch, pull,
    /// rebase, …). The capture service still drains any pending
    /// dirty files; the new variant exists so an empty drain can
    /// still emit a snapshot row that records the new HEAD when the
    /// previous snapshot pointed at a different commit.
    GitRefs,
}

/// Code-quality scan lifecycle phase the bus broadcasts. Mirrors the
/// renderer-era enum.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum CodeQualityScanPhase {
    Started,
    Completed,
    Failed,
}

/// What changed. Variants are deliberately broad — the renderer
/// refetches the affected bucket on receipt rather than trying to
/// reconcile diffs from the payload.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum OxplowEvent {
    /// Any stream row changed (created, renamed, deleted, panes
    /// updated). Renderer refetches `list_streams`.
    StreamsChanged,
    /// The current-stream pointer in `runtime_state` moved.
    CurrentStreamChanged { stream_id: Option<StreamId> },
    /// Threads on `stream_id` changed (created, status flipped, etc.).
    ThreadsChanged { stream_id: StreamId },
    /// Selected-thread pointer for `stream_id` moved.
    SelectedThreadChanged {
        stream_id: StreamId,
        thread_id: Option<ThreadId>,
    },
    /// tasks on `thread_id` (or backlog if `thread_id` is None).
    TasksChanged { thread_id: Option<ThreadId> },
    /// A note was added or removed against an item or thread.
    WorkNotesChanged {
        item_id: Option<TaskId>,
        thread_id: Option<ThreadId>,
    },
    /// A comment (or one of its messages) changed on `target_kind` /
    /// `target_id` within `stream_id`. Renderer refetches the affected
    /// page's comments + the Comments inbox.
    CommentsChanged {
        stream_id: StreamId,
        target_kind: String,
        target_id: String,
    },
    /// A wiki page's backing file changed on disk (creation, body
    /// update, deletion). `slug` is the file stem — subscribers
    /// (e.g. `WikiPageTab`) filter by their own slug so an unrelated
    /// edit doesn't trigger a refresh.
    WikiPagesChanged { slug: String },
    /// Followups for a thread.
    FollowupsChanged { thread_id: ThreadId },
    /// Background task progress.
    BackgroundTasksChanged,
    /// A new hook event landed; renderer refreshes the hook log.
    HookEventsChanged,
    /// Per-thread per-pane agent status changed. `state` carries the
    /// derived status so the renderer can update without a refetch
    /// round-trip — sources that don't have it pre-derived (e.g.
    /// PreToolUse/PostToolUse, where the renderer used to refetch and
    /// re-derive) compute it inline before emitting.
    AgentStatusChanged {
        thread_id: ThreadId,
        pane_target: String,
        state: AgentStatusState,
    },
    /// agent_turn opened or closed.
    AgentTurnsChanged { thread_id: ThreadId },
    /// A page visit was recorded (rail history, recently-finished, etc.).
    /// Coarse — renderer refetches whatever view it cares about.
    PageVisitChanged,
    /// A usage event was recorded. The renderer's filtering uses
    /// `usage_kind` to scope refetches (wiki vs editor-file vs
    /// task, etc.).
    UsageRecorded {
        usage_kind: String,
        key: String,
        stream_id: Option<StreamId>,
        thread_id: Option<ThreadId>,
    },
    /// A file snapshot landed in the snapshot store. Driven by the
    /// background snapshot capture loop or an explicit task event.
    FileSnapshotCreated {
        stream_id: Option<StreamId>,
        snapshot_id: i64,
        source: SnapshotSourceKind,
        effort_id: Option<String>,
        thread_id: Option<ThreadId>,
    },
    /// A batched flush of N file snapshots landed under one parent.
    /// Emitted instead of N per-file `FileSnapshotCreated` events when
    /// `request_snapshot` drains many paths at once (startup sweep,
    /// branch switch). Renderer treats it the same as the per-file
    /// variant — fire-and-forget refetch — so a 34k-file batch causes
    /// one refetch, not 34k.
    FileSnapshotsBatchCreated {
        stream_id: Option<StreamId>,
        snapshot_id: i64,
        file_count: u32,
        source: SnapshotSourceKind,
        effort_id: Option<String>,
        thread_id: Option<ThreadId>,
    },
    /// Effort-scoped collection observations changed for `effort_id`
    /// (a test-run or diff-coverage row landed). The renderer refetches
    /// the effort's observation list. See `.context/collection.md`.
    EffortObservationsChanged {
        thread_id: ThreadId,
        effort_id: String,
    },
    /// `oxplow.yaml` was reloaded from disk (external edit, e.g. the agent
    /// running `/oxplow:configure`). The in-memory config has been swapped;
    /// the renderer refetches `get_config`.
    ConfigChanged,
    /// A code-quality scan transitioned states (started / completed /
    /// failed). The renderer refreshes scan + finding lists on receipt.
    CodeQualityScanned {
        stream_id: Option<StreamId>,
        scan_id: i64,
        tool: String,
        scope: String,
        phase: CodeQualityScanPhase,
    },
    /// `.git` directory appeared/disappeared at the project root —
    /// "is this a git workspace" flipped. Renderer hides/restores the
    /// git-aware UI on receipt.
    WorkspaceContextChanged { git_enabled: bool },
    /// A worktree file changed on disk. Renderer-wide: file tree, quick
    /// open, project panel, git dashboard, uncommitted changes view all
    /// refresh in response.
    WorkspaceChanged {
        stream_id: StreamId,
        change_kind: WorkspaceChangeKind,
        path: String,
    },
    /// A ref under `.git/refs/` changed. Drives history, branch list,
    /// and ahead/behind refreshes. Coarse per stream.
    GitRefsChanged { stream_id: StreamId },
    /// A non-primary stream's backing worktree was deleted out from
    /// under us (externally `rm -rf`'d, `git worktree remove`'d, etc.).
    /// The runtime has already archived the stream by the time this
    /// fires; the renderer surfaces a toast so the user knows why the
    /// rail row vanished. `title` carries the archived stream's display
    /// name.
    StreamOrphaned { stream_id: StreamId, title: String },
}

/// Cheap-to-clone broadcast hub. Capacity is small — subscribers
/// expected to keep up; lagging readers see `RecvError::Lagged` and
/// refetch.
#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<OxplowEvent>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(256);
        Self { sender }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<OxplowEvent> {
        self.sender.subscribe()
    }

    /// Post an event. Returns the number of active receivers (which
    /// may be 0 — that's not an error, the bus is fire-and-forget).
    pub fn emit(&self, event: OxplowEvent) {
        let _ = self.sender.send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscribers_receive_events() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        bus.emit(OxplowEvent::StreamsChanged);
        let got = rx.recv().await.unwrap();
        assert!(matches!(got, OxplowEvent::StreamsChanged));
    }

    #[tokio::test]
    async fn emit_with_no_subscribers_is_noop() {
        let bus = EventBus::new();
        // Should not panic / error.
        bus.emit(OxplowEvent::WikiPagesChanged {
            slug: "test".to_string(),
        });
    }
}
