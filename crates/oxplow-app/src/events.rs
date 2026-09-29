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
    /// ACP session events (`acp::session::AcpEvent`, tsk281).
    pub const ACP: &str = "acp:event";

    /// Frame keys used as `{"channel": <key>, "payload": …}` on the
    /// daemon's multiplexed `/events` socket, keyed to the channel
    /// each frame demuxes back onto.
    pub const FRAMES: &[(&str, &str)] = &[
        ("oxplow", OXPLOW),
        ("lsp", LSP),
        ("terminal", TERMINAL),
        ("acp", ACP),
    ];
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
    /// The set of known language servers changed (Mason package
    /// installed or removed). Renderer refetches `list_lsp_servers`.
    LspServersChanged,
    /// A new hook event landed; renderer refreshes the hook log.
    HookEventsChanged,
    /// Per-thread per-pane agent status changed. `state` carries the
    /// derived status so the renderer can update without a refetch
    /// round-trip — sources that don't have it pre-derived (e.g.
    /// PreToolUse/PostToolUse, where the renderer used to refetch and
    /// re-derive) compute it inline before emitting.
    AgentStatusChanged {
        thread_id: ThreadId,
        state: AgentStatusState,
        /// The status detail, when meaningful to the renderer. Carries
        /// the `await_user` question text when `state` is
        /// `AwaitingUser` so the rail dot's tooltip can show what the
        /// agent is asking — `None` for every other transition.
        detail: Option<String>,
    },
    /// agent_turn opened or closed.
    AgentTurnsChanged { thread_id: ThreadId },
    /// The stall watchdog noticed `thread_id` has in_progress tasks
    /// but its agent has not been running for longer than the alert
    /// threshold — the queue is silently stalled. Emitted once per
    /// stall episode (re-armed when the agent runs again or the
    /// in_progress bucket empties). Renderer surfaces a toast.
    AgentStallAlert {
        thread_id: ThreadId,
        in_progress_count: u32,
        waiting_ms: i64,
    },
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
    /// A snapshot take recorded something new: a new snapshot (its
    /// `file_count` rows), or — `trigger: HeadMoved`, 0 files — the
    /// current snapshot re-stamped with a new HEAD. Emitted after the
    /// take's transaction commits (`SqliteSnapshotStore::record_take`),
    /// never for an unchanged take. The durable record is the
    /// `snapshot_op` row and the `snapshot.taken` / `vcs.head.moved`
    /// event in the log; this is the UI / reactor wake-up.
    SnapshotTaken {
        stream_id: StreamId,
        snapshot_id: i64,
        file_count: u32,
        trigger: oxplow_domain::snapshot::SnapshotTrigger,
        thread_id: Option<ThreadId>,
        turn_id: Option<i64>,
        effort_id: Option<oxplow_domain::EffortId>,
    },
    /// Effort-scoped collection observations changed for `effort_id`
    /// (a test-run or diff-coverage row landed). The renderer refetches
    /// the effort's observation list. See `.context/collection.md`.
    EffortObservationsChanged {
        thread_id: ThreadId,
        effort_id: String,
    },
    /// One or more metric samples landed in `stream_id` (unified metric
    /// substrate, tsk213). `measures` names the measure keys the write touched
    /// so a consumer can skip an event that can't affect it (tsk198); an EMPTY
    /// list is fail-open — "unknown, refresh anyway" — which is what the
    /// low-frequency emit sites still send. See `.context/metrics.md`.
    MetricSamplesChanged {
        stream_id: StreamId,
        #[serde(default)]
        measures: Vec<String>,
    },
    /// A persisted agent nudge landed (report-less-run / coverage-target).
    /// The renderer refetches the effort's (or thread's) nudge list. See
    /// `.context/agent-model.md` (Nudge persistence).
    AgentNudgesChanged {
        thread_id: ThreadId,
        effort_id: Option<String>,
    },
    /// A per-turn agent token-usage row landed (parsed on Stop from the
    /// hook transcript). The renderer refetches the effort's usage list +
    /// the thread's running total. `effort_id` is absent when the Stop had
    /// no open effort. See `.context/agent-model.md` (Token usage capture).
    AgentTokenUsageChanged {
        thread_id: ThreadId,
        effort_id: Option<String>,
    },
    /// `.oxplow/project.yaml` was reloaded from disk (external edit, e.g. the agent
    /// running `/oxplow:configure`). The in-memory config has been swapped;
    /// the renderer refetches `get_config`.
    ConfigChanged,
    /// A user dashboard or one of its tiles was created / edited / reordered /
    /// deleted (tsk138). Project-global (dashboards aren't stream-scoped), so
    /// fieldless — the renderer refetches the affected dashboard(s).
    DashboardsChanged,
    /// An extension source finished a run (ok or error): its entity data
    /// and/or run state changed. Project-global; lenses re-run and the
    /// Extensions settings refresh.
    SourceSynced {
        extension: String,
        source_id: String,
    },
    /// A language server published diagnostics (or restarted) for
    /// `stream_id`: `v_diagnostic` changed. Debounced; lenses re-run.
    DiagnosticsChanged { stream_id: i64 },
    /// A change's analysis landed (`v_change*` for `change_id`).
    ChangeAnalyzed { change_id: i64 },
    /// A stream's working tree or refs moved: its working-tree and
    /// open-effort changes are stale, so pages showing them re-ensure.
    ChangeStale { stream_id: i64 },
    /// An effort's stored metric deltas / observations were recomputed
    /// (`v_effort_metric_delta`, `v_effort_observation`); lenses re-run.
    EffortEvidenceChanged { effort_id: i64 },
    /// Decisions or claims changed for `effort_id` (inferred decisions
    /// stored after an effort closed). Review-packet lenses re-run.
    ReasoningChanged { effort_id: Option<i64> },
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
