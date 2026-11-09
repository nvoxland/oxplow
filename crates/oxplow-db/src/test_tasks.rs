//! Test support: a row in oxplow's `task` table for a test that needs a
//! work item to hang something on (an effort keys on its ref). The task
//! list's store is the `oxplow-tasks` crate's, above this one.

use crate::Database;

/// A `ready` task: its work-item ref.
pub async fn a_task(db: &Database) -> String {
    db.call(move |c| {
        c.execute(
            "INSERT INTO task (title, status, priority, created_by, created_at, updated_at)
             VALUES ('x', 'ready', 'medium', 'user', '2026-01-01T00:00:00.000000Z',
                     '2026-01-01T00:00:00.000000Z')",
            (),
        )?;
        Ok(oxplow_domain::uri::build::work_item_uri(
            "oxplow",
            &format!("tsk{}", c.last_insert_rowid()),
        ))
    })
    .await
    .unwrap()
}
