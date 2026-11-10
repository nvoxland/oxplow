//! The tasks a request worked on (`request_task`, `.context/work-tracking.md`
//! "A task's requests"): observed links, each with its basis, written in the
//! transaction of the evidence. Nothing here moves a task.

use rusqlite::{params, Connection};

use oxplow_domain::{AgentTurnId, DomainError, ThreadId, Timestamp};

use crate::database::{map_sql_err, ts_to_string};

/// A person's prompt in the request named the task.
pub const BASIS_PROMPT: &str = "prompt";
/// The agent ran a task command on it in a turn.
pub const BASIS_COMMAND: &str = "command";
/// It was in progress and assigned to the thread when a turn ended.
pub const BASIS_ASSIGNED: &str = "assigned";

/// Link the request of `turn` to `task` on `basis`. Nothing for a turn with
/// no request, or with no task tracker active; a repeat of the same evidence is the same link.
pub fn link_tx(
    conn: &Connection,
    turn: AgentTurnId,
    task_uri: &str,
    basis: &str,
    at: Timestamp,
) -> Result<(), DomainError> {
    conn.execute(
        "INSERT OR IGNORE INTO request_task (request_id, task_uri, basis, turn_id, at)
         SELECT request_id, ?2, ?3, id, ?4 FROM agent_turn
          WHERE id = ?1 AND request_id IS NOT NULL
            AND EXISTS (SELECT 1 FROM capability_provider
                         WHERE capability = 'task_tracker' AND active = 1)",
        params![turn.value(), task_uri, basis, ts_to_string(at)],
    )
    .map_err(map_sql_err)?;
    Ok(())
}

/// Link the request of `turn` to the tasks in progress and assigned to
/// `thread` on the active task tracker. Nothing when no tracker is active.
pub fn link_assigned_tx(
    conn: &Connection,
    thread: ThreadId,
    turn: AgentTurnId,
    at: Timestamp,
) -> Result<(), DomainError> {
    let assignee = oxplow_domain::uri::build::thread_uri(thread);
    conn.execute(
        "INSERT OR IGNORE INTO request_task (request_id, task_uri, basis, turn_id, at)
         SELECT t.request_id, a.uri, ?3, t.id, ?4
           FROM agent_turn t
           JOIN task_assignment a ON a.assignee = ?2
           JOIN task k ON k.uri = a.uri
           JOIN capability_provider cp
             ON cp.capability = 'task_tracker' AND cp.active = 1 AND cp.provider = k.provider
          WHERE t.id = ?1 AND t.request_id IS NOT NULL
            AND k.deleted_at IS NULL AND k.state = 'in_progress'",
        params![turn.value(), assignee, BASIS_ASSIGNED, ts_to_string(at)],
    )
    .map_err(map_sql_err)?;
    Ok(())
}
