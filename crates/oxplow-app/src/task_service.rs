//! TaskService — orchestration over the Task store.
//!
//! Encapsulates the create/update/reorder/move use-cases. The store
//! itself is a thin row-CRUD layer; everything that requires composing
//! reads and writes (e.g. computing the next sort_index, transitioning
//! status with the associated timestamp side-effects, moving a task
//! between thread and backlog) lives here.
//!
//! The service does not emit events itself — the Tauri command layer
//! does, after a successful service call. That keeps `oxplow-app`
//! independent of the tauri-specta layering and lets the MCP surface
//! reuse the same service without paying for renderer notifications.

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

use oxplow_db::SqliteTaskStore;
use oxplow_db::SqliteThreadStore;
use oxplow_db::{EffortFileChange, SqliteSnapshotStore, SqliteTaskEffortStore, TaskEffortStore};
use oxplow_domain::stores::ThreadStore;
use oxplow_domain::stores::{TaskLinkStore, TaskStore};
use oxplow_domain::EffortId;
use oxplow_domain::{
    DomainError, Task, TaskActorKind, TaskAuthor, TaskId, TaskImpact, TaskLinkType, TaskPriority,
    TaskStatus, ThreadId, Timestamp,
};

#[derive(Debug, Error)]
pub enum TaskServiceError {
    #[error("task not found: {0}")]
    NotFound(TaskId),
    #[error("storage: {0}")]
    Storage(#[from] DomainError),
}

async fn item_is_blocked(
    id: TaskId,
    link_store: &dyn TaskLinkStore,
    by_id: &std::collections::HashMap<TaskId, Task>,
) -> Result<bool, DomainError> {
    let incoming = link_store.list_incoming(id).await?;
    for link in incoming {
        if !matches!(link.link_type, TaskLinkType::Blocks) {
            continue;
        }
        if let Some(blocker) = by_id.get(&link.from_item_id) {
            if !matches!(
                blocker.status,
                TaskStatus::Done | TaskStatus::Canceled | TaskStatus::Archived
            ) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// Discriminated result for `read_task_options`. The shape mirrors
/// main's TS contract so the agent-side skill text stays accurate
/// without a translation layer.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(tag = "mode", rename_all = "snake_case")]
#[allow(clippy::large_enum_variant)]
pub enum ReadWorkOptionsResult {
    Empty,
    Epic { epic: Task, children: Vec<Task> },
    Standalone { items: Vec<Task> },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct CreateTaskInput {
    pub title: String,
    /// The task's prose body (canonical markdown).
    pub description: Option<String>,
    pub parent_id: Option<TaskId>,
    pub status: Option<TaskStatus>,
    pub priority: Option<TaskPriority>,
    pub author: Option<TaskAuthor>,
}

/// Partial-patch for `update_task`. Each `Option` follows
/// "missing -> keep, present -> replace" semantics.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Type)]
pub struct UpdateTaskChanges {
    pub title: Option<String>,
    pub description: Option<String>,
    pub parent_id: Option<Option<TaskId>>,
    pub status: Option<TaskStatus>,
    pub priority: Option<TaskPriority>,
}

#[derive(Clone)]
pub struct TaskService {
    store: Arc<SqliteTaskStore>,
    /// Optional. When set, `update()` opens/closes an effort row on
    /// `in_progress` entry/exit. Held as an `Option` so test paths
    /// that construct a TaskService without the full Services boot
    /// still work — they just skip the lifecycle effort.
    effort_store: Option<Arc<SqliteTaskEffortStore>>,
    /// Per-stream snapshot capture registry. When set alongside
    /// `thread_store`, lifecycle snapshots resolve the right service
    /// via the task's thread → stream — so a task running in a
    /// non-primary worktree captures snapshots against THAT
    /// worktree's fs-watch, not the primary's.
    snapshot_captures: Option<crate::snapshot_capture_registry::SnapshotCaptureRegistry>,
    /// Looks up a thread to read its `stream_id`. Required to drive
    /// the registry-based lookup.
    thread_store: Option<Arc<SqliteThreadStore>>,
}

/// Returns true iff any item in `items` has this id as its parent_id.
fn is_epic(item: &Task, items: &[Task]) -> bool {
    items.iter().any(|c| c.parent_id == Some(item.id))
}

impl TaskService {
    pub fn new(store: Arc<SqliteTaskStore>) -> Self {
        Self {
            store,
            effort_store: None,
            snapshot_captures: None,
            thread_store: None,
        }
    }

    /// Attach the effort store. Required (together with
    /// `with_snapshot_capture`) for automatic effort lifecycle on
    /// in_progress transitions.
    pub fn with_effort_store(mut self, store: Arc<SqliteTaskEffortStore>) -> Self {
        self.effort_store = Some(store);
        self
    }

    /// Attach the per-stream registry. When present alongside
    /// `with_thread_store`, lifecycle snapshots route to the service
    /// matching the task's stream instead of always using the
    /// singleton attached via `with_snapshot_capture`.
    pub fn with_snapshot_captures(
        mut self,
        reg: crate::snapshot_capture_registry::SnapshotCaptureRegistry,
    ) -> Self {
        self.snapshot_captures = Some(reg);
        self
    }

    /// Attach the thread store. Together with `with_snapshot_captures`
    /// this enables per-stream lifecycle snapshots — `TaskService`
    /// loads the task's thread to learn which `stream_id` (and thus
    /// which `SnapshotCaptureService`) to drive.
    pub fn with_thread_store(mut self, store: Arc<SqliteThreadStore>) -> Self {
        self.thread_store = Some(store);
        self
    }

    /// Resolve the snapshot service that should handle a lifecycle
    /// event for `thread_id`. Prefers the per-stream registry when
    /// configured. Returns `None` (and logs) when either the registry
    /// isn't wired or the thread / stream can't be resolved — callers
    /// then skip the snapshot step entirely.
    async fn service_for_thread(
        &self,
        thread_id: &ThreadId,
    ) -> Option<Arc<crate::snapshot_capture::SnapshotCaptureService>> {
        let reg = self.snapshot_captures.as_ref()?;
        let threads = self.thread_store.as_ref()?;
        match threads.get(thread_id).await {
            Ok(Some(thread)) => {
                let svc = reg.get(&thread.stream_id);
                if svc.is_none() {
                    tracing::debug!(
                        thread_id = %thread_id,
                        stream_id = %thread.stream_id,
                        "lifecycle: stream has no registered capture service",
                    );
                }
                svc
            }
            Ok(None) => {
                tracing::debug!(thread_id = %thread_id, "lifecycle: thread row missing");
                None
            }
            Err(e) => {
                tracing::warn!(error = %e, thread_id = %thread_id, "lifecycle: thread lookup failed");
                None
            }
        }
    }

    /// Create a task attached to `thread` (or to the backlog if
    /// `thread` is `None`). Allocates a fresh id and sort_index.
    pub async fn create(
        &self,
        thread: Option<ThreadId>,
        input: CreateTaskInput,
    ) -> Result<Task, TaskServiceError> {
        let next_sort = self.next_sort_index(thread.as_ref()).await?;
        let now = Timestamp::now();
        let mut item = Task {
            // id assigned by store.insert
            id: TaskId::placeholder(),
            thread_id: thread,
            parent_id: input.parent_id,
            title: input.title,
            description: input.description.unwrap_or_default(),
            status: input.status.unwrap_or(TaskStatus::Ready),
            priority: input.priority.unwrap_or(TaskPriority::Medium),
            sort_index: next_sort,
            created_by: TaskActorKind::User,
            created_at: now,
            updated_at: now,
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: input.author.or(Some(TaskAuthor::User)),
        };
        let id = self.store.insert(&item).await?;
        item.id = id;
        // Filing directly in `in_progress` (the path CLAUDE.md
        // recommends to "start the work in the same call") needs the
        // same lifecycle hook that update() runs on a Ready →
        // InProgress transition — otherwise complete_task's EffortEnd
        // snapshot has no open effort to land on and gets orphaned.
        // The insert above already committed, so this is open + backfill
        // (best-effort; boot recovery heals an in_progress task with no
        // open effort).
        if matches!(item.status, TaskStatus::InProgress) {
            if let (Some(effort_store), Some(thread_id)) =
                (self.effort_store.as_ref(), item.thread_id)
            {
                match effort_store.start(item.id, &thread_id, None).await {
                    Ok(eff) => {
                        self.backfill_effort_snapshot(
                            &item,
                            true,
                            oxplow_db::EffortTransition::Opened(eff.id),
                        )
                        .await;
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, task = %item.id, "effort lifecycle: start on create failed");
                    }
                }
            }
        }
        Ok(item)
    }

    /// Apply a partial-patch to an existing task. Returns the
    /// post-patch row.
    pub async fn update(
        &self,
        id: TaskId,
        changes: UpdateTaskChanges,
    ) -> Result<Task, TaskServiceError> {
        let mut item = self.load(id).await?;
        let prior_status = item.status;
        if let Some(t) = changes.title {
            item.title = t;
        }
        if let Some(d) = changes.description {
            item.description = d;
        }
        if let Some(p) = changes.parent_id {
            item.parent_id = p;
        }
        if let Some(s) = changes.status {
            // Transitioning to/from `done` flips completed_at.
            if matches!(s, TaskStatus::Done) && item.status != TaskStatus::Done {
                item.completed_at = Some(Timestamp::now());
            } else if matches!(item.status, TaskStatus::Done) && !matches!(s, TaskStatus::Done) {
                item.completed_at = None;
            }
            item.status = s;
        }
        if let Some(p) = changes.priority {
            item.priority = p;
        }
        item.updated_at = Timestamp::now();

        // Effort lifecycle: when a thread-attached task crosses the
        // `in_progress` boundary, the status flip and the effort
        // open/finish commit as ONE transaction (the invariant
        // "in_progress ⟺ one open effort" can't be torn by a crash),
        // and the snapshot pin is backfilled after commit. The
        // effort-store hook is optional so bare TaskService tests
        // (no Services boot) take the plain-update path.
        let crossed_in =
            prior_status != TaskStatus::InProgress && item.status == TaskStatus::InProgress;
        let crossed_out =
            prior_status == TaskStatus::InProgress && item.status != TaskStatus::InProgress;
        match (
            crossed_in || crossed_out,
            self.effort_store.is_some(),
            item.thread_id,
        ) {
            (true, true, Some(thread_id)) => {
                let transition = self
                    .store
                    .update_with_effort_transition(&item, thread_id, crossed_in)
                    .await?;
                self.backfill_effort_snapshot(&item, crossed_in, transition)
                    .await;
            }
            _ => {
                self.store.update(&item).await?;
            }
        }
        Ok(item)
    }

    /// Post-commit half of the lifecycle transition: request the
    /// EffortStart/EffortEnd snapshot and stamp it onto the effort row
    /// the transaction opened/finished. Best-effort by design — the
    /// effort row is already durable, so a snapshot failure degrades
    /// to a missing pin (no bracket diff), never missing attribution.
    async fn backfill_effort_snapshot(
        &self,
        item: &Task,
        entering: bool,
        transition: oxplow_db::EffortTransition,
    ) {
        let effort_id = match transition {
            oxplow_db::EffortTransition::Opened(id) | oxplow_db::EffortTransition::Finished(id) => {
                id
            }
            oxplow_db::EffortTransition::NoOpenEffort => {
                tracing::debug!(task = %item.id, "effort lifecycle: no open effort to finish");
                return;
            }
        };
        let Some(effort_store) = self.effort_store.as_ref() else {
            return;
        };
        let Some(thread_id) = item.thread_id else {
            return;
        };
        let Some(snapshot) = self.service_for_thread(&thread_id).await else {
            return;
        };
        let source = if entering {
            crate::events::SnapshotSourceKind::EffortStart
        } else {
            crate::events::SnapshotSourceKind::EffortEnd
        };
        let snap_id = match snapshot.request_snapshot(source).await {
            Ok(Some(id)) => id,
            Ok(None) => return,
            Err(e) => {
                tracing::warn!(error = %e, task = %item.id, "effort lifecycle: snapshot failed");
                return;
            }
        };
        let stamp = if entering {
            effort_store.set_start_snapshot(&effort_id, snap_id).await
        } else {
            effort_store.set_end_snapshot(&effort_id, snap_id).await
        };
        if let Err(e) = stamp {
            tracing::warn!(error = %e, task = %item.id, "effort lifecycle: snapshot backfill failed");
        }
    }

    /// Rewrite sort_index across the items in `thread` (or backlog if
    /// `thread` is None) according to the supplied order. Items not
    /// included keep their existing sort_index.
    pub async fn reorder(
        &self,
        thread: Option<&ThreadId>,
        order: &[TaskId],
    ) -> Result<(), TaskServiceError> {
        let now = Timestamp::now();
        for (idx, id) in order.iter().enumerate() {
            let mut item = self.load(*id).await?;
            // Only reorder items in the right scope.
            if item.thread_id.as_ref() != thread {
                continue;
            }
            item.sort_index = idx as i64;
            item.updated_at = now;
            self.store.update(&item).await?;
        }
        Ok(())
    }

    /// Move a task to a different thread (or to the backlog with
    /// `dest = None`). Reallocates sort_index at the destination tail.
    pub async fn move_to(
        &self,
        id: TaskId,
        dest: Option<ThreadId>,
    ) -> Result<Task, TaskServiceError> {
        let mut item = self.load(id).await?;
        let next_sort = self.next_sort_index(dest.as_ref()).await?;
        item.thread_id = dest;
        item.sort_index = next_sort;
        item.updated_at = Timestamp::now();
        self.store.update(&item).await?;
        Ok(item)
    }

    pub async fn list_for_thread(&self, thread: &ThreadId) -> Result<Vec<Task>, TaskServiceError> {
        Ok(self.store.list_for_thread(thread).await?)
    }

    /// Open + record + close an effort for `item` against `thread`.
    /// Declared `impacts` are persisted before finish so the
    /// page_ref projection runs once with the full payload.
    ///
    /// `worktree_root`, when supplied, lets the store classify each
    /// touched file as `Deleted` (file no longer on disk) vs.
    /// `Updated` (file still present). Without a baseline snapshot
    /// "Created" can't be distinguished from "Updated" by stat
    /// alone, so callers needing that signal should declare it via
    /// `impacts` (`{kind:"file", action:"created"}`). Pass `None`
    /// from tests / callers that don't have a worktree handle — the
    /// store falls back to `Updated` for every path, matching the
    /// pre-change behavior.
    // Each parameter is doing distinct semantic work — bundling
    // into a struct would hide that without buying anything.
    #[allow(clippy::too_many_arguments)]
    pub async fn record_effort(
        &self,
        effort_store: &SqliteTaskEffortStore,
        item: TaskId,
        thread: &ThreadId,
        touched_files: &[String],
        summary: Option<String>,
        impacts: &[TaskImpact],
        worktree_root: Option<&Path>,
    ) -> Result<(), TaskServiceError> {
        // Resolve the version triple from the most-recent effort
        // BEFORE the transaction — it reads the snapshot store. The
        // attribution itself (attach-or-start + files + impacts +
        // finish/summary) commits as one transaction, so a crash can
        // no longer leave files recorded without their summary/finish.
        let version = match effort_store.most_recent_for_task(item).await? {
            Some(e) => self.resolve_effort_file_version(&e).await,
            // No effort yet — the atomic op will open one with no
            // snapshot pin, so the version triple is the unpinned
            // default.
            None => crate::file_ref_version::ResolvedFileVersion {
                local_snapshot_id: 0,
                closest_git_version: None,
                git_version_exact: false,
            },
        };
        let files: Vec<(String, oxplow_db::EffortFileChange)> = touched_files
            .iter()
            .filter(|p| !p.is_empty())
            .map(|p| (p.clone(), classify_change(worktree_root, p)))
            .collect();
        effort_store
            .record_effort_atomic(oxplow_db::RecordEffortAtomic {
                task: item,
                thread: *thread,
                files,
                version: oxplow_db::OwnedFileRefVersion {
                    local_snapshot_id: version.local_snapshot_id,
                    closest_git_version: version.closest_git_version,
                    git_version_exact: version.git_version_exact,
                },
                impacts: impacts.to_vec(),
                summary,
            })
            .await?;
        Ok(())
    }

    /// Auto-claim a single file the agent just edited onto the thread's
    /// OPEN effort, in real time from the PostToolUse hook (Child 1 of the
    /// claim-first attribution epic). Idempotent — `record_file` is
    /// `INSERT OR REPLACE` keyed on `(effort_id, path)`, so the agent's
    /// `touched_files` at completion merely confirms/amends rather than
    /// enumerating from scratch. Returns `Ok(true)` when a claim was
    /// recorded, `Ok(false)` when no effort is open (no-op). Best-effort:
    /// the PostToolUse caller swallows errors so the hook never fails.
    pub async fn claim_open_effort_file(
        &self,
        effort_store: &SqliteTaskEffortStore,
        thread: &ThreadId,
        path: &str,
        worktree_root: Option<&Path>,
    ) -> Result<bool, TaskServiceError> {
        if path.is_empty() {
            return Ok(false);
        }
        let Some(effort) = effort_store.find_open_for_thread(thread).await? else {
            return Ok(false);
        };
        let version = self.resolve_effort_file_version(&effort).await;
        let change = classify_change(worktree_root, path);
        effort_store
            .record_file(&effort.id, path, change, version.as_ref())
            .await?;
        Ok(true)
    }

    pub async fn list_backlog(&self) -> Result<Vec<Task>, TaskServiceError> {
        Ok(self.store.list_backlog().await?)
    }

    /// Pin the local snapshot id used by the effort's file-ref rows
    /// and resolve its closest git commit. Falls back to a 0
    /// snapshot id when neither end nor start is set (rare —
    /// only happens for an effort opened without a snapshot pin and
    /// no snapshot service attached). The cascade in
    /// `set_snapshot_git_commit` will retroactively flip
    /// `git_version_exact` to true if a commit lands on the chosen
    /// snapshot later.
    pub async fn resolve_effort_file_version(
        &self,
        effort: &oxplow_db::TaskEffort,
    ) -> crate::file_ref_version::ResolvedFileVersion {
        let snapshot_id = effort
            .end_snapshot_id
            .or(effort.start_snapshot_id)
            .unwrap_or(0);
        let svc = self.service_for_thread(&effort.thread_id).await;
        match svc {
            Some(svc) if snapshot_id != 0 => {
                crate::file_ref_version::resolve(svc.store(), svc.project_dir(), snapshot_id)
                    .await
                    .unwrap_or(crate::file_ref_version::ResolvedFileVersion {
                        local_snapshot_id: snapshot_id,
                        closest_git_version: None,
                        git_version_exact: false,
                    })
            }
            _ => crate::file_ref_version::ResolvedFileVersion {
                local_snapshot_id: snapshot_id,
                closest_git_version: None,
                git_version_exact: false,
            },
        }
    }
}

/// Set-wise diff between what the agent claimed in `touched_files`
/// and what the snapshot bracket actually shows changed during the
/// effort. Returned alongside the task on `complete_task` and
/// surfaced via the Stop hook so the agent can choose to amend.
/// Skipped (None) entirely when the auto-diff matches the claim, or
/// when no snapshot bracket is available (effort has no start/end
/// snapshot pin yet).
#[derive(Debug, Clone, serde::Serialize)]
pub struct EffortFileReview {
    pub effort_id: String,
    pub task_id: i64,
    /// Paths the agent claimed but the auto-diff doesn't see as
    /// changed. Disclaim via `amend_effort(remove_files=…)` if not
    /// actually touched.
    pub claimed_but_not_changed: Vec<String>,
    /// Paths the auto-diff sees as changed but the agent didn't
    /// claim. Capped at `MAX_UNCLAIMED_FOR_REVIEW`; when larger,
    /// the field is empty and `unclaimed_overflow` is set.
    pub changed_but_not_claimed: Vec<String>,
    /// Number of changed-but-not-claimed paths the diff actually
    /// contained, before any cap was applied. `None` means the
    /// list is the full set.
    pub unclaimed_overflow: Option<usize>,
}

/// Cap on the "files in the diff that the agent didn't claim" list
/// surfaced to the agent. Above this volume something else is
/// happening (overlapping efforts, formatter, codegen, user edits)
/// and the agent can't be expected to triage a wall of paths.
pub const MAX_UNCLAIMED_FOR_REVIEW: usize = 10;

/// Compare the agent's declared `touched_files` for a task's
/// most-recent effort against the auto-diff between
/// start_snapshot_id and end_snapshot_id. Returns `None` when
/// nothing's worth showing the agent — claim and diff agree, or no
/// snapshot bracket exists yet.
pub async fn compute_effort_file_review(
    effort_store: &SqliteTaskEffortStore,
    snapshot_store: &SqliteSnapshotStore,
    task_id: TaskId,
    claimed: &[String],
) -> Option<EffortFileReview> {
    let effort = effort_store
        .most_recent_for_task(task_id)
        .await
        .ok()
        .flatten()?;
    let changed = effort_changed_paths(snapshot_store, &effort).await?;
    let acknowledged = effort_store
        .list_acknowledged_paths(&effort.id)
        .await
        .ok()?;
    let other_claimed = effort_store
        .paths_claimed_by_intervening_efforts(&effort.id)
        .await
        .ok()?;
    review_from_lists(
        &effort.id,
        task_id,
        claimed,
        &changed,
        &acknowledged,
        &other_claimed,
    )
}

/// The set of paths whose content changed between the effort's start
/// and end snapshots, via the shared snapshot diff (content/hash-based,
/// not snapshot-row membership). `None` if the effort has no snapshot
/// bracket yet.
async fn effort_changed_paths(
    snapshot_store: &SqliteSnapshotStore,
    effort: &oxplow_db::TaskEffort,
) -> Option<Vec<String>> {
    let (start, end) = (effort.start_snapshot_id?, effort.end_snapshot_id?);
    let changes = snapshot_store.diff_snapshots(Some(start), end).await.ok()?;
    Some(changes.into_iter().map(|c| c.path).collect())
}

/// Recompute a review for a specific effort id. The Stop hook
/// uses this to refresh a stale review after the agent may have
/// called `amend_effort`. Returns `None` when the effort no longer
/// has a discrepancy (or doesn't exist / has no snapshot bracket).
pub async fn recompute_effort_file_review(
    effort_store: &SqliteTaskEffortStore,
    snapshot_store: &SqliteSnapshotStore,
    effort_id: &EffortId,
) -> Option<EffortFileReview> {
    let effort = effort_store.get_effort(effort_id).await.ok().flatten()?;
    let files = effort_store.list_files(effort_id).await.ok()?;
    let claimed: Vec<String> = files.iter().map(|f| f.path.clone()).collect();
    let changed = effort_changed_paths(snapshot_store, &effort).await?;
    let acknowledged = effort_store.list_acknowledged_paths(effort_id).await.ok()?;
    let other_claimed = effort_store
        .paths_claimed_by_intervening_efforts(effort_id)
        .await
        .ok()?;
    review_from_lists(
        effort_id,
        effort.task_id,
        &claimed,
        &changed,
        &acknowledged,
        &other_claimed,
    )
}

fn review_from_lists(
    effort_id: &EffortId,
    task_id: TaskId,
    claimed: &[String],
    changed: &[String],
    acknowledged: &[String],
    other_claimed: &[String],
) -> Option<EffortFileReview> {
    let claimed_set: std::collections::HashSet<&str> = claimed.iter().map(|s| s.as_str()).collect();
    let changed_set: std::collections::HashSet<&str> = changed.iter().map(|s| s.as_str()).collect();
    let acknowledged_set: std::collections::HashSet<&str> =
        acknowledged.iter().map(|s| s.as_str()).collect();
    let other_claimed_set: std::collections::HashSet<&str> =
        other_claimed.iter().map(|s| s.as_str()).collect();
    let mut claimed_but_not_changed: Vec<String> = claimed_set
        .difference(&changed_set)
        .map(|s| (*s).to_string())
        .collect();
    // A changed path another effort already claimed (it finished inside
    // this effort's window) isn't this agent's to claim — drop it.
    let mut changed_but_not_claimed: Vec<String> = changed_set
        .difference(&claimed_set)
        .filter(|s| !acknowledged_set.contains(*s) && !other_claimed_set.contains(*s))
        .map(|s| (*s).to_string())
        .collect();
    claimed_but_not_changed.sort();
    changed_but_not_claimed.sort();
    if claimed_but_not_changed.is_empty() && changed_but_not_claimed.is_empty() {
        return None;
    }
    let overflow = if changed_but_not_claimed.len() > MAX_UNCLAIMED_FOR_REVIEW {
        let total = changed_but_not_claimed.len();
        changed_but_not_claimed.clear();
        Some(total)
    } else {
        None
    };
    Some(EffortFileReview {
        effort_id: effort_id.to_string(),
        task_id: task_id.value(),
        claimed_but_not_changed,
        changed_but_not_claimed,
        unclaimed_overflow: overflow,
    })
}

impl TaskService {
    /// Return the next dispatch unit for the orchestrator.
    pub async fn read_task_options(
        &self,
        thread: &ThreadId,
        link_store: &dyn TaskLinkStore,
    ) -> Result<ReadWorkOptionsResult, TaskServiceError> {
        let all = self.store.list_for_thread(thread).await?;
        let by_id: std::collections::HashMap<TaskId, Task> =
            all.iter().map(|i| (i.id, i.clone())).collect();

        let mut ready: Vec<Task> = all
            .iter()
            .filter(|i| i.status == TaskStatus::Ready)
            .cloned()
            .collect();
        ready.sort_by_key(|i| (i.sort_index, i.created_at));

        let mut unblocked_ready: Vec<Task> = Vec::new();
        for item in &ready {
            if !item_is_blocked(item.id, link_store, &by_id).await? {
                unblocked_ready.push(item.clone());
            }
        }

        let Some(head) = unblocked_ready.first().cloned() else {
            return Ok(ReadWorkOptionsResult::Empty);
        };

        if is_epic(&head, &all) {
            let mut children: Vec<Task> = Vec::new();
            let mut frontier = vec![head.id];
            while let Some(parent_id) = frontier.pop() {
                for it in &all {
                    if it.parent_id == Some(parent_id) {
                        if it.status == TaskStatus::Ready
                            && !item_is_blocked(it.id, link_store, &by_id).await?
                        {
                            children.push(it.clone());
                        }
                        frontier.push(it.id);
                    }
                }
            }
            children.sort_by_key(|i| (i.sort_index, i.created_at));
            return Ok(ReadWorkOptionsResult::Epic {
                epic: head,
                children,
            });
        }

        let standalone: Vec<Task> = unblocked_ready
            .into_iter()
            .filter(|i| !is_epic(i, &all))
            .collect();
        Ok(ReadWorkOptionsResult::Standalone { items: standalone })
    }

    pub async fn soft_delete(&self, id: TaskId) -> Result<(), TaskServiceError> {
        self.store.soft_delete(id).await?;
        Ok(())
    }

    async fn load(&self, id: TaskId) -> Result<Task, TaskServiceError> {
        self.store
            .get(id)
            .await?
            .ok_or(TaskServiceError::NotFound(id))
    }

    async fn next_sort_index(&self, thread: Option<&ThreadId>) -> Result<i64, TaskServiceError> {
        let items = match thread {
            Some(t) => self.store.list_for_thread(t).await?,
            None => self.store.list_backlog().await?,
        };
        Ok(items.iter().map(|i| i.sort_index).max().unwrap_or(-1) + 1)
    }
}

/// Classify how a path changed during an effort by stat-ing the
/// worktree. Without a baseline snapshot we can't reliably tell
/// "created" apart from "updated" (the agent might have edited a
/// pre-existing file too), so this returns:
///
///  - `Deleted` if the file is missing on disk now
///  - `Updated` if the file is present (the dominant case)
///
/// Agents that want explicit "created" attribution should declare
/// it via the `impacts` parameter on `complete_task`. Returns
/// `Updated` when `worktree_root` is `None` so test fixtures that
/// don't carry a real worktree keep their old behavior.
fn classify_change(worktree_root: Option<&Path>, path: &str) -> EffortFileChange {
    let Some(root) = worktree_root else {
        return EffortFileChange::Updated;
    };
    let resolved = root.join(path);
    match std::fs::symlink_metadata(&resolved) {
        Ok(_) => EffortFileChange::Updated,
        Err(_) => EffortFileChange::Deleted,
    }
}

/// The bucketed view the Backlog page renders.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct BacklogState {
    pub items: Vec<Task>,
    pub waiting: Vec<Task>,
    pub in_progress: Vec<Task>,
    pub done: Vec<Task>,
}

impl BacklogState {
    pub fn from_rows(rows: Vec<Task>) -> Self {
        let mut items = Vec::new();
        let mut waiting = Vec::new();
        let mut in_progress = Vec::new();
        let mut done = Vec::new();
        for r in rows {
            match r.status {
                TaskStatus::InProgress => in_progress.push(r),
                TaskStatus::Done | TaskStatus::Canceled | TaskStatus::Archived => done.push(r),
                TaskStatus::Blocked => waiting.push(r),
                TaskStatus::Ready => items.push(r),
            }
        }
        Self {
            items,
            waiting,
            in_progress,
            done,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::{Database, SqliteStreamStore, SqliteThreadStore};
    use oxplow_domain::stores::{StreamStore, ThreadStore};
    use oxplow_domain::{Stream, StreamId, StreamKind, Thread, ThreadStatus};

    #[test]
    fn classify_change_defaults_to_updated_without_worktree() {
        // No worktree → caller (test or a path that hasn't plumbed
        // the root yet) gets the same behavior as before the
        // detection landed.
        assert_eq!(
            classify_change(None, "src/anything.rs"),
            EffortFileChange::Updated
        );
    }

    #[test]
    fn classify_change_detects_deletion() {
        let tmp = tempfile::tempdir().unwrap();
        // File doesn't exist → Deleted.
        assert_eq!(
            classify_change(Some(tmp.path()), "missing.rs"),
            EffortFileChange::Deleted
        );
        // File exists → Updated (we can't tell created from
        // modified without a baseline snapshot).
        let real = tmp.path().join("real.rs");
        std::fs::write(&real, "fn main() {}").unwrap();
        assert_eq!(
            classify_change(Some(tmp.path()), "real.rs"),
            EffortFileChange::Updated
        );
    }

    #[test]
    fn review_subtracts_acknowledged_paths_from_unclaimed() {
        // Diff sees `extra.rs` plus `claimed.rs`; agent only claimed
        // `claimed.rs`. Without ack: `extra.rs` shows in
        // `changed_but_not_claimed`. With ack: it's filtered out
        // and the review collapses to `None`.
        let effort = EffortId::new(1);
        let task = TaskId::new(1);
        let claimed = vec!["claimed.rs".to_string()];
        let changed = vec!["claimed.rs".to_string(), "extra.rs".to_string()];
        let no_ack = review_from_lists(&effort, task, &claimed, &changed, &[], &[]);
        let r = no_ack.expect("unclaimed extra.rs should produce a review");
        assert_eq!(r.changed_but_not_claimed, vec!["extra.rs".to_string()]);
        let with_ack = review_from_lists(
            &effort,
            task,
            &claimed,
            &changed,
            &["extra.rs".to_string()],
            &[],
        );
        assert!(
            with_ack.is_none(),
            "acknowledged path should clear the discrepancy: {with_ack:?}",
        );
        // Same effect when another effort already claimed the path.
        let with_other = review_from_lists(
            &effort,
            task,
            &claimed,
            &changed,
            &[],
            &["extra.rs".to_string()],
        );
        assert!(
            with_other.is_none(),
            "path claimed by an intervening effort should clear the discrepancy: {with_other:?}",
        );
    }

    #[test]
    fn classify_change_treats_symlink_as_present() {
        // Even a broken symlink reports via symlink_metadata, so the
        // path is "present" from the agent's point of view —
        // resolving the link is a deletion concern.
        let tmp = tempfile::tempdir().unwrap();
        let link = tmp.path().join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink("nowhere", &link).unwrap();
        #[cfg(not(unix))]
        {
            let _ = link;
            return;
        }
        assert_eq!(
            classify_change(Some(tmp.path()), "link"),
            EffortFileChange::Updated
        );
    }

    async fn fixture() -> (TaskService, ThreadId) {
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db.clone());
        let store = Arc::new(SqliteTaskStore::new(db));
        let s = Stream {
            id: StreamId::new(1),
            kind: StreamKind::Primary,
            title: "p".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/p".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        };
        streams.upsert(&s).await.unwrap();
        let t = Thread {
            id: ThreadId::new(1),
            stream_id: s.id,
            title: "x".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            agent: oxplow_domain::AgentKind::Claude,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        };
        threads.upsert(&t).await.unwrap();
        (TaskService::new(store), t.id)
    }

    async fn fixture_with_lifecycle() -> (
        TaskService,
        ThreadId,
        Arc<SqliteTaskEffortStore>,
        tempfile::TempDir,
        crate::snapshot_capture_registry::SnapshotCaptureRegistry,
    ) {
        let project = tempfile::tempdir().unwrap();
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db.clone());
        let task_store = Arc::new(SqliteTaskStore::new(db.clone()));
        let effort_store = Arc::new(SqliteTaskEffortStore::new(db.clone()));
        let snapshot_store = Arc::new(oxplow_db::SqliteSnapshotStore::new(db.clone()));
        let blobs = crate::blob_store::BlobStore::new(project.path().join(".oxplow/snapshots"));
        let s = Stream {
            id: StreamId::new(1),
            kind: StreamKind::Primary,
            title: "p".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: project.path().to_string_lossy().into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        };
        streams.upsert(&s).await.unwrap();
        // Build a single-entry registry pointing at the test project's
        // worktree. Per-stream is the registry's whole point — the
        // primary stream IS the only stream in these tests, but
        // TaskService routes through `get(&stream_id)` either way.
        let event_bus = crate::events::EventBus::new();
        let snapshot_captures = crate::snapshot_capture_registry::SnapshotCaptureRegistry::new(
            crate::snapshot_capture_registry::SnapshotCaptureRegistryConfig {
                snapshot_store: snapshot_store.clone(),
                blobs: blobs.clone(),
                max_file_bytes: 1_000_000,
                workspace_filter: oxplow_fs_watch::WorkspaceFilter::default(),
                events: event_bus,
            },
        );
        // Drop the default-built service; tests need overridden
        // settle / predrain durations, so we re-insert a custom one
        // below. Both gates are independently covered in
        // `snapshot_capture::tests`.
        let _ = snapshot_captures
            .register(&s)
            .expect("test stream's worktree exists on disk");
        let snapshot_svc = Arc::new(
            crate::snapshot_capture::SnapshotCaptureService::new(
                snapshot_store,
                blobs,
                project.path().to_path_buf(),
                s.id,
                1_000_000,
                oxplow_fs_watch::WorkspaceFilter::default(),
            )
            .with_settle_duration(std::time::Duration::ZERO)
            .with_predrain_delay(std::time::Duration::ZERO),
        );
        snapshot_captures.unregister(&s.id);
        snapshot_captures.insert_for_test(s.id, snapshot_svc);
        snapshot_captures.set_primary(s.id);
        let t = Thread {
            id: ThreadId::new(2),
            stream_id: s.id,
            title: "t".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            agent: oxplow_domain::AgentKind::Claude,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        };
        threads.upsert(&t).await.unwrap();
        let thread_store_for_svc = Arc::new(oxplow_db::SqliteThreadStore::new(db.clone()));
        let svc = TaskService::new(task_store)
            .with_effort_store(effort_store.clone())
            .with_snapshot_captures(snapshot_captures.clone())
            .with_thread_store(thread_store_for_svc);
        (svc, t.id, effort_store, project, snapshot_captures)
    }

    #[tokio::test]
    async fn in_progress_transition_opens_effort_with_start_snapshot() {
        let (svc, tid, effort_store, _project, captures) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "lifecycle".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Ready → InProgress: opens an effort with start_snapshot_id.
        let _ = svc
            .update(
                item.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let open = effort_store
            .find_open_for_task(item.id)
            .await
            .unwrap()
            .expect("effort should be open");
        // Dirty set is empty in tests (no actual fs writes), so the
        // first snapshot returns None. The effort still opens but
        // start_snapshot_id is None — that's the "nothing to pin"
        // case and is fine. To verify the snapshot path actually
        // ran, write a file first.
        assert!(open.ended_at.is_none());
        assert!(open.start_snapshot_id.is_none());

        // Mark a file dirty so the next request_snapshot produces
        // a non-empty result.
        let svc_for_dirty = captures
            .primary()
            .expect("primary service registered in fixture");
        std::fs::write(_project.path().join("a.txt"), "v").unwrap();
        svc_for_dirty.mark_dirty(
            _project.path().join("a.txt"),
            oxplow_fs_watch::WatchEventKind::Other,
        );

        // InProgress → Done: closes the open effort with end_snapshot_id.
        let _ = svc
            .update(
                item.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let efforts = effort_store.list_for_item(item.id).await.unwrap();
        assert_eq!(efforts.len(), 1);
        let closed = &efforts[0];
        assert!(closed.ended_at.is_some());
        assert!(closed.end_snapshot_id.is_some());
        // And no new effort was opened.
        assert!(effort_store
            .find_open_for_task(item.id)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn create_with_in_progress_opens_lifecycle_effort() {
        // Filing a task directly in `in_progress` (the path CLAUDE.md
        // recommends for "start the work in the same call") must run
        // the lifecycle hook — otherwise complete_task's TaskEnd
        // snapshot has no open effort to attach to and the snapshot
        // is orphaned.
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "born running".into(),
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let open = effort_store
            .find_open_for_task(item.id)
            .await
            .unwrap()
            .expect("lifecycle effort should be open after in_progress create");
        assert!(open.ended_at.is_none());
    }

    #[tokio::test]
    async fn create_with_done_skips_effort_lifecycle() {
        // Filing directly in a terminal status (e.g. retroactively
        // logging completed work) must NOT open a lifecycle effort —
        // record_effort handles that synthesis itself, with the
        // touched_files payload.
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "retro".into(),
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(effort_store
            .find_open_for_task(item.id)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn record_effort_merges_into_lifecycle_effort() {
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "merge".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // Open the lifecycle effort.
        let _ = svc
            .update(
                item.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // Close it.
        let _ = svc
            .update(
                item.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // Now record_effort comes in with touched files + summary.
        // It should attach to the already-closed lifecycle effort,
        // NOT create a second row.
        svc.record_effort(
            &effort_store,
            item.id,
            &tid,
            &["src/x.rs".to_string()],
            Some("did the thing".into()),
            &[],
            None,
        )
        .await
        .unwrap();
        let efforts = effort_store.list_for_item(item.id).await.unwrap();
        assert_eq!(efforts.len(), 1, "should still be a single effort row");
        let row = &efforts[0];
        assert_eq!(row.summary.as_deref(), Some("did the thing"));
        let files = effort_store.list_files(&row.id).await.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "src/x.rs");
    }

    #[tokio::test]
    async fn record_effort_creates_fresh_effort_when_no_lifecycle() {
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "direct".into(),
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // No lifecycle ran — task filed directly as done.
        svc.record_effort(
            &effort_store,
            item.id,
            &tid,
            &["a.rs".to_string()],
            Some("retro".into()),
            &[],
            None,
        )
        .await
        .unwrap();
        let efforts = effort_store.list_for_item(item.id).await.unwrap();
        assert_eq!(efforts.len(), 1);
        assert!(efforts[0].ended_at.is_some());
        assert_eq!(efforts[0].summary.as_deref(), Some("retro"));
    }

    #[tokio::test]
    async fn claim_open_effort_file_claims_on_open_effort_and_is_idempotent() {
        // Auto-claim (PostToolUse path) records a task_effort_file on the
        // thread's open effort, and a repeat claim of the same path is
        // idempotent (INSERT OR REPLACE → still one row).
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "claim".into(),
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let open = effort_store
            .find_open_for_task(item.id)
            .await
            .unwrap()
            .expect("effort open");

        let claimed = svc
            .claim_open_effort_file(&effort_store, &tid, "src/edited.rs", None)
            .await
            .unwrap();
        assert!(claimed, "a claim should be recorded on the open effort");
        // Idempotent: claiming the same path again doesn't duplicate.
        let again = svc
            .claim_open_effort_file(&effort_store, &tid, "src/edited.rs", None)
            .await
            .unwrap();
        assert!(again);
        let files = effort_store.list_files(&open.id).await.unwrap();
        assert_eq!(files.len(), 1, "idempotent — one row");
        assert_eq!(files[0].path, "src/edited.rs");
    }

    #[tokio::test]
    async fn claim_open_effort_file_no_open_effort_is_noop() {
        // No open effort on the thread → the auto-claim is a no-op
        // (returns false, records nothing).
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let claimed = svc
            .claim_open_effort_file(&effort_store, &tid, "src/edited.rs", None)
            .await
            .unwrap();
        assert!(!claimed, "no open effort → no claim");
    }

    #[tokio::test]
    async fn non_in_progress_transitions_skip_effort_lifecycle() {
        let (svc, tid, effort_store, _project, _captures) = fixture_with_lifecycle().await;
        let item = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "skip".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // Ready → Blocked: no effort row.
        let _ = svc
            .update(
                item.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::Blocked),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(effort_store
            .list_for_item(item.id)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn create_assigns_increasing_sort_index() {
        let (svc, tid) = fixture().await;
        let a = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "a".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let b = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "b".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(a.sort_index, 0);
        assert_eq!(b.sort_index, 1);
    }

    #[tokio::test]
    async fn update_title_keeps_other_fields() {
        let (svc, tid) = fixture().await;
        let it = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "before".into(),
                    description: Some("desc".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let updated = svc
            .update(
                it.id,
                UpdateTaskChanges {
                    title: Some("after".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(updated.title, "after");
        assert_eq!(updated.description, "desc");
    }

    #[tokio::test]
    async fn transition_to_done_sets_completed_at() {
        let (svc, tid) = fixture().await;
        let it = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "x".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(it.completed_at.is_none());
        let done = svc
            .update(
                it.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(done.completed_at.is_some());
        let reopened = svc
            .update(
                done.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(reopened.completed_at.is_none());
    }

    #[tokio::test]
    async fn move_to_backlog_clears_thread_id_and_resorts() {
        let (svc, tid) = fixture().await;
        let it = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "x".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let moved = svc.move_to(it.id, None).await.unwrap();
        assert!(moved.thread_id.is_none());
        let bl = svc.list_backlog().await.unwrap();
        assert_eq!(bl.len(), 1);
        assert_eq!(bl[0].id, it.id);
    }

    #[tokio::test]
    async fn reorder_rewrites_indices() {
        let (svc, tid) = fixture().await;
        let a = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "a".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let b = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "b".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let c = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "c".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        // c, a, b
        svc.reorder(Some(&tid), &[c.id, a.id, b.id]).await.unwrap();
        let list = svc.list_for_thread(&tid).await.unwrap();
        let order: Vec<_> = list.iter().map(|i| i.id).collect();
        assert_eq!(order, vec![c.id, a.id, b.id]);
    }

    #[test]
    fn backlog_state_buckets_by_status() {
        let now = Timestamp::from_unix_ms(1);
        let mk = |id: i64, status| Task {
            id: TaskId::new(id),
            thread_id: None,
            parent_id: None,
            title: id.to_string(),
            description: String::new(),
            status,
            priority: TaskPriority::Medium,
            sort_index: 0,
            created_by: TaskActorKind::User,
            created_at: now,
            updated_at: now,
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(TaskAuthor::User),
        };
        let rows = vec![
            mk(1, TaskStatus::Ready),
            mk(2, TaskStatus::InProgress),
            mk(3, TaskStatus::Done),
            mk(4, TaskStatus::Blocked),
        ];
        let st = BacklogState::from_rows(rows);
        assert_eq!(st.items.len(), 1);
        assert_eq!(st.in_progress.len(), 1);
        assert_eq!(st.done.len(), 1);
        assert_eq!(st.waiting.len(), 1);
    }

    #[test]
    fn backlog_state_collapses_canceled_and_archived_into_done() {
        let now = Timestamp::from_unix_ms(1);
        let mk = |id: i64, status| Task {
            id: TaskId::new(id),
            thread_id: None,
            parent_id: None,
            title: id.to_string(),
            description: String::new(),
            status,
            priority: TaskPriority::Medium,
            sort_index: 0,
            created_by: TaskActorKind::User,
            created_at: now,
            updated_at: now,
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: Some(TaskAuthor::User),
        };
        let st = BacklogState::from_rows(vec![
            mk(1, TaskStatus::Done),
            mk(2, TaskStatus::Canceled),
            mk(3, TaskStatus::Archived),
        ]);
        assert_eq!(st.done.len(), 3);
        assert!(st.items.is_empty());
        assert!(st.in_progress.is_empty());
        assert!(st.waiting.is_empty());
    }

    #[test]
    fn backlog_state_empty_input() {
        let st = BacklogState::from_rows(vec![]);
        assert!(
            st.items.is_empty()
                && st.waiting.is_empty()
                && st.in_progress.is_empty()
                && st.done.is_empty()
        );
    }

    // ---- read_task_options edge cases ----

    async fn link_store_fixture() -> (TaskService, oxplow_db::SqliteTaskLinkStore, ThreadId) {
        let db = Database::in_memory();
        let streams = SqliteStreamStore::new(db.clone());
        let threads = SqliteThreadStore::new(db.clone());
        let store = Arc::new(SqliteTaskStore::new(db.clone()));
        let link_store = oxplow_db::SqliteTaskLinkStore::new(db.clone());
        let s = Stream {
            id: StreamId::new(1),
            kind: StreamKind::Primary,
            title: "p".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: "/p".into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        };
        streams.upsert(&s).await.unwrap();
        let t = Thread {
            id: ThreadId::new(1),
            stream_id: s.id,
            title: "x".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            agent: oxplow_domain::AgentKind::Claude,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        };
        threads.upsert(&t).await.unwrap();
        (TaskService::new(store), link_store, t.id)
    }

    #[tokio::test]
    async fn read_work_options_empty_when_no_ready_items() {
        let (svc, links, tid) = link_store_fixture().await;
        let a = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "in flight".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        svc.update(
            a.id,
            UpdateTaskChanges {
                status: Some(TaskStatus::InProgress),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let result = svc.read_task_options(&tid, &links).await.unwrap();
        assert!(matches!(result, ReadWorkOptionsResult::Empty));
    }

    #[tokio::test]
    async fn read_work_options_returns_standalone_for_plain_task() {
        let (svc, links, tid) = link_store_fixture().await;
        svc.create(
            Some(tid),
            CreateTaskInput {
                title: "ready task".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let result = svc.read_task_options(&tid, &links).await.unwrap();
        match result {
            ReadWorkOptionsResult::Standalone { items } => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].title, "ready task");
            }
            other => panic!("expected Standalone, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_work_options_returns_epic_with_ready_children() {
        let (svc, links, tid) = link_store_fixture().await;
        let epic = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "the epic".into(),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let _child_a = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "child A".into(),
                    parent_id: Some(epic.id),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let _child_b = svc
            .create(
                Some(tid),
                CreateTaskInput {
                    title: "child B".into(),
                    parent_id: Some(epic.id),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let result = svc.read_task_options(&tid, &links).await.unwrap();
        match result {
            ReadWorkOptionsResult::Epic { epic: e, children } => {
                assert_eq!(e.id, epic.id);
                assert_eq!(children.len(), 2);
            }
            other => panic!("expected Epic, got {other:?}"),
        }
    }

    /// Regression: a task running on a non-primary stream must capture
    /// snapshots against THAT stream's worktree. Before the per-stream
    /// registry, the lifecycle would always hit the primary's
    /// fs-watcher, leaving the bracket diff empty for any edit landing
    /// in a worktree-stream.
    #[tokio::test]
    async fn non_primary_stream_lifecycle_captures_against_its_own_worktree() {
        // Two on-disk directories — one for the "primary" stream and a
        // separate one for the worktree stream. Both write file_snapshot
        // rows to the same DB but the rows are tagged per-stream.
        let primary_dir = tempfile::tempdir().unwrap();
        let worktree_dir = tempfile::tempdir().unwrap();
        let db = Database::in_memory();
        let stream_store = SqliteStreamStore::new(db.clone());
        let thread_store_handle = SqliteThreadStore::new(db.clone());
        let task_store = Arc::new(SqliteTaskStore::new(db.clone()));
        let effort_store = Arc::new(SqliteTaskEffortStore::new(db.clone()));
        let snapshot_store = Arc::new(oxplow_db::SqliteSnapshotStore::new(db.clone()));
        let blobs = crate::blob_store::BlobStore::new(primary_dir.path().join(".oxplow/snapshots"));

        let primary = Stream {
            id: StreamId::new(1),
            kind: StreamKind::Primary,
            title: "p".into(),
            branch: "main".into(),
            branch_ref: "refs/heads/main".into(),
            branch_source: "main".into(),
            worktree_path: primary_dir.path().to_string_lossy().into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(1),
            updated_at: Timestamp::from_unix_ms(1),
            archived_at: None,
        };
        let worktree = Stream {
            id: StreamId::new(2),
            kind: StreamKind::Worktree,
            title: "feature".into(),
            branch: "feature".into(),
            branch_ref: "refs/heads/feature".into(),
            branch_source: "main".into(),
            worktree_path: worktree_dir.path().to_string_lossy().into(),
            working_pane: String::new(),
            talking_pane: String::new(),
            working_session_id: String::new(),
            talking_session_id: String::new(),
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(2),
            updated_at: Timestamp::from_unix_ms(2),
            archived_at: None,
        };
        stream_store.upsert(&primary).await.unwrap();
        stream_store.upsert(&worktree).await.unwrap();

        let snapshot_captures = crate::snapshot_capture_registry::SnapshotCaptureRegistry::new(
            crate::snapshot_capture_registry::SnapshotCaptureRegistryConfig {
                snapshot_store: snapshot_store.clone(),
                blobs: blobs.clone(),
                max_file_bytes: 1_000_000,
                workspace_filter: oxplow_fs_watch::WorkspaceFilter::default(),
                events: crate::events::EventBus::new(),
            },
        );
        // Register both streams the same way Services::boot does, then
        // swap each entry for a settle/predrain-zero variant so tests
        // don't burn the debounce windows.
        for s in [&primary, &worktree] {
            snapshot_captures.register(s).expect("worktree dir exists");
            snapshot_captures.unregister(&s.id);
            let svc = Arc::new(
                crate::snapshot_capture::SnapshotCaptureService::new(
                    snapshot_store.clone(),
                    blobs.clone(),
                    std::path::PathBuf::from(&s.worktree_path),
                    s.id,
                    1_000_000,
                    oxplow_fs_watch::WorkspaceFilter::default(),
                )
                .with_settle_duration(std::time::Duration::ZERO)
                .with_predrain_delay(std::time::Duration::ZERO),
            );
            snapshot_captures.insert_for_test(s.id, svc);
        }
        snapshot_captures.set_primary(primary.id);

        // Thread is on the WORKTREE stream — this is the case the bug
        // was about.
        let thread = Thread {
            id: ThreadId::new(3),
            stream_id: worktree.id,
            title: "t".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
            agent: oxplow_domain::AgentKind::Claude,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: Timestamp::from_unix_ms(3),
            updated_at: Timestamp::from_unix_ms(3),
            archived_at: None,
        };
        thread_store_handle.upsert(&thread).await.unwrap();

        let svc = TaskService::new(task_store)
            .with_effort_store(effort_store.clone())
            .with_snapshot_captures(snapshot_captures.clone())
            .with_thread_store(Arc::new(SqliteThreadStore::new(db.clone())));

        // Seed a baseline snapshot in the worktree stream so the
        // EffortStart capture has something to anchor `start_snapshot_id`
        // against (an empty dirty set returns the latest-existing id,
        // which would otherwise be NULL on a fresh DB).
        let seed = worktree_dir.path().join("seed.txt");
        std::fs::write(&seed, "baseline").unwrap();
        let worktree_svc_pre = snapshot_captures.get(&worktree.id).unwrap();
        worktree_svc_pre.mark_dirty(seed, oxplow_fs_watch::WatchEventKind::Other);
        let _ = worktree_svc_pre
            .request_snapshot(crate::events::SnapshotSourceKind::Startup)
            .await
            .unwrap();

        // File the task in_progress — that opens an effort + captures
        // start_snapshot_id.
        let item = svc
            .create(
                Some(thread.id),
                CreateTaskInput {
                    title: "fix something in the worktree".into(),
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        // Edit a file in the WORKTREE stream's directory and mark it
        // dirty against the worktree stream's service.
        let edited = worktree_dir.path().join("changed.txt");
        std::fs::write(&edited, "hello").unwrap();
        let worktree_svc = snapshot_captures
            .get(&worktree.id)
            .expect("worktree service registered");
        worktree_svc.mark_dirty(edited.clone(), oxplow_fs_watch::WatchEventKind::Other);
        // Also mark something dirty against the primary's service.
        // This file would have been captured under the old code too —
        // we're asserting it does NOT show up in the worktree's effort.
        let primary_edit = primary_dir.path().join("other.txt");
        std::fs::write(&primary_edit, "ignored").unwrap();
        let primary_svc = snapshot_captures
            .get(&primary.id)
            .expect("primary service registered");
        primary_svc.mark_dirty(primary_edit.clone(), oxplow_fs_watch::WatchEventKind::Other);

        // Done: closes the effort and captures the end snapshot —
        // routes through the worktree stream's service because the
        // task's thread.stream_id == worktree.id.
        let _ = svc
            .update(
                item.id,
                UpdateTaskChanges {
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        let closed = effort_store
            .list_for_item(item.id)
            .await
            .unwrap()
            .into_iter()
            .next()
            .expect("one effort recorded");
        assert!(closed.ended_at.is_some());
        assert!(closed.start_snapshot_id.is_some());
        assert!(closed.end_snapshot_id.is_some());

        let changed =
            oxplow_db::TaskEffortStore::list_changed_paths_for_effort(&*effort_store, &closed.id)
                .await
                .unwrap();
        assert!(
            changed.iter().any(|p| p == "changed.txt"),
            "worktree edit must be visible in the bracket diff; got {changed:?}",
        );
        assert!(
            !changed.iter().any(|p| p == "other.txt"),
            "primary-stream edit must NOT bleed into the worktree's effort; got {changed:?}",
        );
    }
}
