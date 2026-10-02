//! Task domain types.
//!
//! The task is the durable unit of authored change in oxplow — every
//! Edit/Write the agent makes must trace back to one. Tasks form a
//! parent/child tree: an "epic" is any task that has children.

use serde::{Deserialize, Serialize};
use specta::Type;

use crate::ids::{TaskId, TaskLinkId, ThreadId};
use crate::time::Timestamp;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Ready,
    InProgress,
    Blocked,
    Done,
    Canceled,
    Archived,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    Low,
    Medium,
    High,
    Urgent,
}

/// Who or what wrote a task row to the DB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TaskActorKind {
    User,
    Agent,
    System,
}

/// Semantic origin — distinct from `created_by` (the writer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "snake_case")]
pub enum TaskAuthor {
    User,
    Agent,
}

/// The relationship type between two tasks.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum TaskLinkType {
    Blocks,
    RelatesTo,
    DiscoveredFrom,
    Duplicates,
    Supersedes,
    RepliesTo,
}

/// A task row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Task {
    pub id: TaskId,
    /// `None` when the task is on the project-wide backlog.
    pub thread_id: Option<ThreadId>,
    pub parent_id: Option<TaskId>,
    pub title: String,
    /// The task's prose body — the canonical markdown detail.
    pub description: String,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    pub sort_index: i64,
    pub created_by: TaskActorKind,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub completed_at: Option<Timestamp>,
    pub deleted_at: Option<Timestamp>,
    pub note_count: i64,
    pub author: Option<TaskAuthor>,
}

impl Task {
    /// Move to `to` at `now`: `completed_at` is set on entering `done` and
    /// cleared on moving anywhere but `done` or `archived` — archiving is
    /// tidying, so an archived task still says whether (and when) it was
    /// completed; `updated_at` moves.
    pub fn set_status(&mut self, to: TaskStatus, now: Timestamp) {
        if to == TaskStatus::Done && self.status != TaskStatus::Done {
            self.completed_at = Some(now);
        } else if to != TaskStatus::Done && to != TaskStatus::Archived {
            self.completed_at = None;
        }
        self.status = to;
        self.updated_at = now;
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct TaskLink {
    pub id: TaskLinkId,
    pub thread_id: ThreadId,
    pub from_item_id: TaskId,
    pub to_item_id: TaskId,
    pub link_type: TaskLinkType,
    pub created_at: Timestamp,
}

/// External-input length caps, kept here so both the validation layer
/// and the DB layer reach for the same constants.
pub mod limits {
    pub const TITLE_MAX_LEN: usize = 500;
    pub const DESCRIPTION_MAX_LEN: usize = 20_000;
    pub const NOTE_MAX_LEN: usize = 20_000;
}

/// Sentinel scope used for the project-wide backlog (tasks not
/// attached to any thread).
pub const BACKLOG_SCOPE: &str = "__backlog__";

/// A note attached to either a task or a thread (mutually exclusive —
/// enforced at the DB CHECK constraint).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct TaskNote {
    pub id: crate::ids::NoteId,
    pub task_id: Option<TaskId>,
    pub thread_id: Option<ThreadId>,
    pub body: String,
    pub author: String,
    pub created_at: Timestamp,
}

/// One declared cross-page outcome of an effort — the LLM asserts
/// "this effort created/updated/deleted/referenced/resolved <kind>:<id>".
/// Stored as a JSON list on `effort.impacts_json` and projected
/// into the unified `page_ref` graph as outbound edges from the
/// owning task. Distinct from `touched_files` (which only covers
/// the file kind) — impacts cover wiki pages, tasks, commits,
/// findings, directories, and files alike.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Type, schemars::JsonSchema)]
pub struct TaskImpact {
    /// Page kind being impacted — `wiki | task | file | directory
    /// | git_commit | finding`. Stored snake-case on the wire,
    /// normalized to the canonical `page_ref` kinds at projection
    /// time (`git_commit` → `commit`, `directory` → `dir`, `task` →
    /// `work_item`; see `normalize_impact_kind`).
    pub kind: String,
    /// Canonical id for that page kind (slug, integer string, repo
    /// path, sha — see `page_ref_projections` docs).
    pub id: String,
    /// What the effort did. Free-form but conventionally one of
    /// `created | updated | deleted | referenced | resolved |
    /// completed | reopened`. Persisted in `source_extra` so the
    /// UI can render it without re-querying.
    pub action: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archiving_a_done_task_keeps_when_it_was_completed() {
        let at = |ms| Timestamp::from_unix_ms(ms);
        let mut task = Task {
            id: TaskId::new(1),
            thread_id: None,
            parent_id: None,
            title: "t".into(),
            description: String::new(),
            status: TaskStatus::Ready,
            priority: TaskPriority::Medium,
            sort_index: 0,
            created_by: TaskActorKind::User,
            created_at: at(1),
            updated_at: at(1),
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: None,
        };
        task.set_status(TaskStatus::Done, at(2));
        task.set_status(TaskStatus::Archived, at(3));
        assert_eq!(
            task.completed_at,
            Some(at(2)),
            "archiving is tidying, not undoing"
        );
        task.set_status(TaskStatus::Ready, at(4));
        assert_eq!(task.completed_at, None, "reopened: no longer completed");
        task.set_status(TaskStatus::Archived, at(5));
        assert_eq!(task.completed_at, None);
    }

    #[test]
    fn enum_round_trips_as_snake_case() {
        let ip = TaskStatus::InProgress;
        let json = serde_json::to_string(&ip).unwrap();
        assert_eq!(json, "\"in_progress\"");
        let back: TaskStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(ip, back);
    }

    #[test]
    fn link_type_uses_snake_case_in_json() {
        let lt = TaskLinkType::DiscoveredFrom;
        let json = serde_json::to_string(&lt).unwrap();
        assert_eq!(json, "\"discovered_from\"");
    }

    #[test]
    fn task_round_trips() {
        let now = Timestamp::from_unix_ms(1_700_000_000_000);
        let item = Task {
            id: TaskId::new(1),
            thread_id: Some(ThreadId::new(1)),
            parent_id: None,
            title: "ship it".into(),
            description: String::new(),
            status: TaskStatus::Ready,
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

        let json = serde_json::to_string(&item).unwrap();
        let back: Task = serde_json::from_str(&json).unwrap();
        assert_eq!(item, back);
    }

    #[test]
    fn backlog_task_has_no_thread() {
        let item: Task = serde_json::from_str(
            r#"{
                "id":"tsk7","thread_id":null,"parent_id":null,
                "title":"t","description":"",
                "status":"ready","priority":"medium","sort_index":0,
                "created_by":"user",
                "created_at":"2026-04-29T12:00:00Z","updated_at":"2026-04-29T12:00:00Z",
                "completed_at":null,"deleted_at":null,"note_count":0,
                "author":"user"
            }"#,
        )
        .unwrap();
        assert!(item.thread_id.is_none());
    }
}
