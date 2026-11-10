//! oxplow's task-list types: a task, its status and priority, its links
//! and notes. Nothing outside this crate names them: everything else
//! reads the task interface (`.context/tasks.md`). Tasks form a
//! parent/child tree: an "epic" is any task that has children.

use serde::{Deserialize, Serialize};

use oxplow_domain::{NoteId, Timestamp};

use crate::ids::OxtaskId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OxtaskStatus {
    /// Filed, not ready to work on yet.
    Backlog,
    Ready,
    InProgress,
    Blocked,
    Done,
    Canceled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OxtaskPriority {
    Low,
    Medium,
    High,
    Urgent,
}

/// Who or what wrote a task row to the DB.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OxtaskActorKind {
    User,
    Agent,
    System,
}

/// Semantic origin — distinct from `created_by` (the writer).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OxtaskAuthor {
    User,
    Agent,
}

/// The relationship of a link from a task: to another item, or (`about`)
/// to what the task is about — any ref.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OxtaskLinkType {
    Blocks,
    RelatesTo,
    DiscoveredFrom,
    Duplicates,
    Supersedes,
    RepliesTo,
    About,
}

/// A task row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Oxtask {
    pub id: OxtaskId,
    pub parent_id: Option<OxtaskId>,
    pub title: String,
    /// The task's prose body — the canonical markdown detail.
    pub description: String,
    pub status: OxtaskStatus,
    pub priority: OxtaskPriority,
    pub created_by: OxtaskActorKind,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub completed_at: Option<Timestamp>,
    pub deleted_at: Option<Timestamp>,
    pub note_count: i64,
    pub author: Option<OxtaskAuthor>,
}

impl Oxtask {
    /// Move to `to` at `now`: `completed_at` is set on entering `done` and
    /// cleared on leaving it; `updated_at` moves.
    pub fn set_status(&mut self, to: OxtaskStatus, now: Timestamp) {
        if to == OxtaskStatus::Done && self.status != OxtaskStatus::Done {
            self.completed_at = Some(now);
        } else if to != OxtaskStatus::Done {
            self.completed_at = None;
        }
        self.status = to;
        self.updated_at = now;
    }
}

/// A comment on an oxplow task (`oxtask_note:<id>`; a `oxplow.task.comment`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OxtaskNote {
    pub id: NoteId,
    pub task_id: OxtaskId,
    pub body: String,
    pub author: String,
    pub created_at: Timestamp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_says_when_it_was_completed_and_reopening_clears_it() {
        let at = |ms| Timestamp::from_unix_ms(ms);
        let mut task = Oxtask {
            id: OxtaskId::new(1),
            parent_id: None,
            title: "t".into(),
            description: String::new(),
            status: OxtaskStatus::Ready,
            priority: OxtaskPriority::Medium,
            created_by: OxtaskActorKind::User,
            created_at: at(1),
            updated_at: at(1),
            completed_at: None,
            deleted_at: None,
            note_count: 0,
            author: None,
        };
        task.set_status(OxtaskStatus::Done, at(2));
        assert_eq!(task.completed_at, Some(at(2)));
        task.set_status(OxtaskStatus::Ready, at(4));
        assert_eq!(task.completed_at, None, "reopened: no longer completed");
    }

    #[test]
    fn enum_round_trips_as_snake_case() {
        let ip = OxtaskStatus::InProgress;
        let json = serde_json::to_string(&ip).unwrap();
        assert_eq!(json, "\"in_progress\"");
        let back: OxtaskStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(ip, back);
    }

    #[test]
    fn link_type_uses_snake_case_in_json() {
        let lt = OxtaskLinkType::DiscoveredFrom;
        let json = serde_json::to_string(&lt).unwrap();
        assert_eq!(json, "\"discovered_from\"");
    }

    #[test]
    fn a_backlog_task_is_a_status() {
        let item: Oxtask = serde_json::from_str(
            r#"{
                "id":"tsk7","parent_id":null,
                "title":"t","description":"",
                "status":"backlog","priority":"medium",
                "created_by":"user",
                "created_at":"2026-04-29T12:00:00Z","updated_at":"2026-04-29T12:00:00Z",
                "completed_at":null,"deleted_at":null,"note_count":0,
                "author":"user"
            }"#,
        )
        .unwrap();
        assert_eq!(item.status, OxtaskStatus::Backlog);
    }
}
