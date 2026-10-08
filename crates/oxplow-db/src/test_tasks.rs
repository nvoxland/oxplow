//! Test support: a row in oxplow's `task` table for a test that needs a
//! work item to hang something on (an effort keys on its ref). The task
//! list's store is the `oxplow-tasks` crate's, above this one.

use oxplow_domain::{TaskId, ThreadId};

use crate::Database;

/// A `ready` task on `thread` (the backlog when `None`).
pub async fn a_task(db: &Database, thread: Option<ThreadId>) -> TaskId {
    db.call(move |c| {
        c.execute(
            "INSERT INTO task (thread_id, title, status, priority, created_by, created_at, updated_at)
             VALUES (?1, 'x', 'ready', 'medium', 'user', '2026-01-01T00:00:00.000000Z',
                     '2026-01-01T00:00:00.000000Z')",
            [thread.map(|t| t.value())],
        )?;
        Ok(TaskId::new(c.last_insert_rowid()))
    })
    .await
    .unwrap()
}

/// A task's work-item ref.
pub fn work_item_ref(task: TaskId) -> String {
    format!("work_item:oxplow:{task}")
}
