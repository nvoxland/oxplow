//! How oxplow's task fields map onto the work-item interface: its
//! statuses onto the canonical states (and back), its priority as the
//! one native field it takes.

use oxplow_domain::work_items::CanonicalState;
use oxplow_domain::CommandError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{Task, TaskPriority, TaskStatus};

/// oxplow's status for a canonical state.
pub fn native_status(state: CanonicalState) -> TaskStatus {
    match state {
        CanonicalState::Todo => TaskStatus::Ready,
        CanonicalState::InProgress => TaskStatus::InProgress,
        CanonicalState::Blocked => TaskStatus::Blocked,
        CanonicalState::Done => TaskStatus::Done,
        CanonicalState::Canceled => TaskStatus::Canceled,
    }
}

/// The canonical state of a task: `ready` is `todo`; `archived` is `done`
/// when it was completed, else `canceled` (the same mapping its
/// `work_item` row is projected with).
pub fn canonical_of(task: &Task) -> CanonicalState {
    state_pair(task.status, task.completed_at.is_some()).0
}

/// The `state` / `native_state` pair that names `status`: `archived`
/// rides on `done` when the task was completed (`completed`), else on
/// `canceled`.
pub fn state_pair(status: TaskStatus, completed: bool) -> (CanonicalState, String) {
    let state = match status {
        TaskStatus::Ready => CanonicalState::Todo,
        TaskStatus::InProgress => CanonicalState::InProgress,
        TaskStatus::Blocked => CanonicalState::Blocked,
        TaskStatus::Done => CanonicalState::Done,
        TaskStatus::Canceled => CanonicalState::Canceled,
        TaskStatus::Archived if completed => CanonicalState::Done,
        TaskStatus::Archived => CanonicalState::Canceled,
    };
    (state, status_str(status))
}

/// A status as its wire string (`in_progress`).
pub fn status_str(status: TaskStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .expect("a status serializes as a string")
}

/// oxplow's status for a `state` / `native_state` pair (either or both;
/// `None` when neither is given). A `native_state` must be an oxplow
/// status and, with a `state`, map to it — `archived` to `done` or
/// `canceled`, the rest by name — else `Invalid` at `/native_state`.
pub fn oxplow_status(
    state: Option<CanonicalState>,
    native_state: Option<&str>,
) -> Result<Option<TaskStatus>, CommandError> {
    let invalid = |message: String| CommandError::Invalid {
        field: Some("/native_state".into()),
        message,
    };
    match (state, native_state) {
        (None, None) => Ok(None),
        (Some(state), None) => Ok(Some(native_status(state))),
        (state, Some(raw)) => {
            let status: TaskStatus =
                serde_json::from_value(Value::String(raw.into())).map_err(|_| {
                    invalid(format!(
                        "`{raw}` isn't an oxplow status (ready, in_progress, blocked, done, \
                         canceled, archived)"
                    ))
                })?;
            if let Some(state) = state {
                let fits = match status {
                    TaskStatus::Archived => {
                        matches!(state, CanonicalState::Done | CanonicalState::Canceled)
                    }
                    other => native_status(state) == other,
                };
                if !fits {
                    return Err(invalid(format!(
                        "`{raw}` isn't an oxplow status for `{}`",
                        state.as_str()
                    )));
                }
            }
            Ok(Some(status))
        }
    }
}

/// oxplow's own fields, under `native`: a task's priority.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OxplowNative {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<TaskPriority>,
}

/// The `native` an input carries, as oxplow's fields.
pub fn oxplow_native(native: Option<&Value>) -> Result<OxplowNative, CommandError> {
    match native {
        None => Ok(OxplowNative::default()),
        Some(v) => serde_json::from_value(v.clone()).map_err(|e| CommandError::Invalid {
            field: Some("/native".into()),
            message: format!("oxplow's native field is `priority`: {e}"),
        }),
    }
}
