//! oxplow's task-list types: a task, its status and priority, its links
//! and notes. Nothing outside this crate names them: everything else
//! reads the work-item interface (`.context/work-items.md`). Tasks form a
//! parent/child tree: an "epic" is any task that has children.

use serde::{Deserialize, Serialize};

use oxplow_domain::{NoteId, TaskId, TaskLinkId, ThreadId, Timestamp};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Ready,
    InProgress,
    Blocked,
    Done,
    Canceled,
    Archived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    Low,
    Medium,
    High,
    Urgent,
}

/// Who or what wrote a task row to the DB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskActorKind {
    User,
    Agent,
    System,
}

/// Semantic origin — distinct from `created_by` (the writer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskAuthor {
    User,
    Agent,
}

/// The relationship type between two tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskLink {
    pub id: TaskLinkId,
    pub thread_id: ThreadId,
    pub from_item_id: TaskId,
    pub to_item_id: TaskId,
    pub link_type: TaskLinkType,
    pub created_at: Timestamp,
}

/// A comment on an oxplow task (`task_note:<id>`; a `oxplow.work_item.comment`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskNote {
    pub id: NoteId,
    pub task_id: TaskId,
    pub body: String,
    pub author: String,
    pub created_at: Timestamp,
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
