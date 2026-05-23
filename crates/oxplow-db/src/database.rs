use std::path::Path;
use std::sync::Arc;

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use tracing::info;

mod embedded {
    refinery::embed_migrations!("migrations");
}

/// Pooled SQLite connection used by every store impl in this crate.
///
/// Constructed once at app startup; `Arc` it and hand it to each
/// store. Connections are obtained via `pool.get()`; DB calls run
/// inside `tokio::task::spawn_blocking` from the service layer so
/// the synchronous rusqlite API doesn't block the async runtime.
#[derive(Clone)]
pub struct Database {
    pool: Arc<Pool<SqliteConnectionManager>>,
}

impl Database {
    /// Open (or create) the SQLite file at `path` and apply migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DbInitError> {
        let manager = SqliteConnectionManager::file(path.as_ref()).with_init(|c| {
            c.pragma_update(None, "journal_mode", "WAL")?;
            c.pragma_update(None, "foreign_keys", "ON")?;
            c.pragma_update(None, "synchronous", "NORMAL")?;
            Ok(())
        });
        let pool = Pool::builder()
            .max_size(8)
            .build(manager)
            .map_err(DbInitError::Pool)?;

        let mut conn = pool.get().map_err(DbInitError::Pool)?;
        embedded::migrations::runner()
            .run(&mut *conn)
            .map_err(|e| DbInitError::Migration(e.to_string()))?;
        info!("oxplow db opened at {}", path.as_ref().display());

        Ok(Self {
            pool: Arc::new(pool),
        })
    }

    /// In-memory DB for tests. Each call returns a fresh DB.
    ///
    /// Public so other crates' tests can build a Services graph
    /// without needing a tempfile.
    pub fn in_memory() -> Self {
        let manager = SqliteConnectionManager::memory().with_init(|c| {
            c.pragma_update(None, "foreign_keys", "ON")?;
            Ok(())
        });
        let pool = Pool::builder()
            .max_size(1)
            .build(manager)
            .expect("in-memory sqlite pool builds");
        let mut conn = pool.get().expect("in-memory sqlite connection");
        embedded::migrations::runner()
            .run(&mut *conn)
            .expect("in-memory migrations run");
        Self {
            pool: Arc::new(pool),
        }
    }

    /// Borrow a connection from the pool. Most stores should call this
    /// inside a `spawn_blocking` so the synchronous rusqlite API doesn't
    /// stall the tokio runtime.
    pub(crate) fn conn(
        &self,
    ) -> Result<r2d2::PooledConnection<SqliteConnectionManager>, r2d2::Error> {
        self.pool.get()
    }

    /// Best-effort connection-pool drain. Useful at app shutdown so
    /// SQLite file handles release before we exit; under normal Drop
    /// the pool's connections close lazily.
    ///
    /// Note: this only works while no other `Arc<Database>` clones
    /// hold connections — by definition, nothing checked out from the
    /// pool. Call from the daemon shutdown path after services have
    /// been told to stop.
    pub fn close(&self) {
        // r2d2 doesn't expose a public drain API. We can flush the
        // pool by setting an aggressive max_idle_lifetime on a clone,
        // but the simplest correct thing is to let Drop handle it.
        // This method exists as a hook for callers who want to be
        // explicit about shutdown ordering — in practice it's a
        // no-op today but reserves the API contract.
        tracing::debug!("oxplow db close requested");
    }

    /// Run a closure with a borrowed connection. Pure convenience
    /// wrapper that maps pool errors into `oxplow_domain::DomainError`.
    pub(crate) fn with_conn<R>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<R>,
    ) -> Result<R, oxplow_domain::DomainError> {
        let conn = self
            .conn()
            .map_err(|e| oxplow_domain::DomainError::Invalid(format!("pool: {e}")))?;
        f(&conn).map_err(|e| oxplow_domain::DomainError::Invalid(format!("sql: {e}")))
    }

    /// Run a blocking DB closure off the async runtime. Wraps
    /// `spawn_blocking` + [`Self::with_conn`] and flattens the
    /// `JoinError` (a panicked blocking task) into a `DomainError`
    /// instead of unwrapping it. Store methods should prefer this over
    /// hand-rolling the `spawn_blocking(move || db.with_conn(…)).await
    /// .unwrap()` dance.
    pub(crate) async fn call<R, F>(&self, f: F) -> Result<R, oxplow_domain::DomainError>
    where
        F: FnOnce(&Connection) -> rusqlite::Result<R> + Send + 'static,
        R: Send + 'static,
    {
        let db = self.clone();
        tokio::task::spawn_blocking(move || db.with_conn(f))
            .await
            .map_err(|e| oxplow_domain::DomainError::Invalid(format!("db task panicked: {e}")))?
    }

    /// Like [`Self::call`] but hands the closure a `&mut Connection`, for
    /// the few stores that need a `rusqlite::Transaction` (which borrows
    /// the connection mutably). The closure returns a `DomainError`
    /// directly — transaction methods already map their own SQL errors —
    /// rather than the `rusqlite::Result` `call` expects.
    pub(crate) async fn call_mut<R, F>(&self, f: F) -> Result<R, oxplow_domain::DomainError>
    where
        F: FnOnce(&mut Connection) -> Result<R, oxplow_domain::DomainError> + Send + 'static,
        R: Send + 'static,
    {
        let db = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = db
                .conn()
                .map_err(|e| oxplow_domain::DomainError::Invalid(format!("pool: {e}")))?;
            f(&mut conn)
        })
        .await
        .map_err(|e| oxplow_domain::DomainError::Invalid(format!("db task panicked: {e}")))?
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DbInitError {
    #[error("connection pool: {0}")]
    Pool(r2d2::Error),
    #[error("migration: {0}")]
    Migration(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_db_runs_migrations() {
        let db = Database::in_memory();
        let conn = db.conn().unwrap();
        // Sanity check: the streams table exists after migrations.
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name='streams'")
            .unwrap();
        let row: String = stmt.query_row([], |r| r.get(0)).unwrap();
        assert_eq!(row, "streams");
    }

    #[test]
    fn runtime_state_seeded() {
        let db = Database::in_memory();
        let conn = db.conn().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM runtime_state WHERE id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }

    /// Schema regression: every table that should exist after all
    /// migrations apply should be present. agent_status + hook_event
    /// were dropped in V2 (now in-memory) — assert they're GONE so a
    /// future migration accidentally re-adding them fails this test.
    #[test]
    fn migrations_produce_expected_table_set() {
        let db = Database::in_memory();
        let conn = db.conn().unwrap();
        let expected_present = [
            "streams",
            "runtime_state",
            "threads",
            "thread_selection",
            "task_note",
            "agent_turn",
            "wiki_page",
            "page_visit",
            "usage_event",
            "code_quality_scan",
            "code_quality_finding",
            "file_snapshot",
            "snapshot",
            "task",
            "task_link",
            "task_event",
            "task_commit",
            "task_effort",
            "task_effort_file",
            "task_effort_turn",
            "wiki_page_thread_update",
            "page_ref",
            "comment",
            "comment_message",
        ];
        let expected_absent = ["hook_event", "agent_status"];
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        let actual: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .filter_map(|r| r.ok())
            // refinery's migration tracking table is internal noise.
            .filter(|n| !n.starts_with("refinery_"))
            // sqlite's autoindex bookkeeping isn't a "table" we care about.
            .filter(|n| !n.starts_with("sqlite_"))
            .collect();
        for table in expected_present {
            assert!(
                actual.iter().any(|a| a == table),
                "expected table `{table}` to exist; got: {actual:?}"
            );
        }
        for table in expected_absent {
            assert!(
                !actual.iter().any(|a| a == table),
                "expected table `{table}` to be DROPPED; got: {actual:?}"
            );
        }
    }

    /// The unique-primary-stream and unique-active-thread invariants
    /// must be enforced by partial indexes, not just by the
    /// application layer. A direct INSERT bypassing the stores should
    /// still fail.
    #[test]
    fn primary_stream_uniqueness_enforced_at_db() {
        let db = Database::in_memory();
        let conn = db.conn().unwrap();
        let now = "2026-04-29T00:00:00Z";
        conn.execute(
            "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
             VALUES ('s-a', 'primary', 'a', 'main', 'refs/heads/main', 'main', '/r', ?1, ?1)",
            [now],
        ).unwrap();
        let result = conn.execute(
            "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
             VALUES ('s-b', 'primary', 'b', 'main', 'refs/heads/main', 'main', '/r', ?1, ?1)",
            [now],
        );
        assert!(result.is_err(), "DB should reject a second primary stream");
    }

    #[test]
    fn active_thread_uniqueness_enforced_at_db() {
        let db = Database::in_memory();
        let conn = db.conn().unwrap();
        let now = "2026-04-29T00:00:00Z";
        conn.execute(
            "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
             VALUES ('s-1', 'primary', 'a', 'main', 'refs/heads/main', 'main', '/r', ?1, ?1)",
            [now],
        ).unwrap();
        conn.execute(
            "INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
             VALUES ('b-a', 's-1', 'a', 'active', ?1, ?1)",
            [now],
        )
        .unwrap();
        let result = conn.execute(
            "INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
             VALUES ('b-b', 's-1', 'b', 'active', ?1, ?1)",
            [now],
        );
        assert!(
            result.is_err(),
            "DB should reject a second active thread on the same stream"
        );
    }

    #[test]
    fn foreign_keys_enabled() {
        let db = Database::in_memory();
        let conn = db.conn().unwrap();
        let result = conn.execute(
            "INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
             VALUES ('b-orphan', 's-nope', 't', 'queued', '2026-01-01', '2026-01-01')",
            [],
        );
        assert!(
            result.is_err(),
            "FK enforcement must be on so dangling stream_id is rejected"
        );
    }

    #[test]
    fn work_note_xor_invariant_enforced() {
        let db = Database::in_memory();
        let conn = db.conn().unwrap();
        // Setup minimal parent rows.
        let now = "2026-04-29T00:00:00Z";
        conn.execute(
            "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
             VALUES ('s-1', 'primary', 'a', 'main', 'r', 'r', '/r', ?1, ?1)",
            [now],
        ).unwrap();
        conn.execute(
            "INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
             VALUES ('b-1', 's-1', 't', 'active', ?1, ?1)",
            [now],
        )
        .unwrap();
        // Both null — must fail.
        let r = conn.execute(
            "INSERT INTO task_note (id, body, author, created_at) VALUES ('n-bad', 'b', 'u', ?1)",
            [now],
        );
        assert!(
            r.is_err(),
            "task_note with neither parent should fail CHECK"
        );
        // Both set — must fail.
        let r = conn.execute(
            "INSERT INTO task (title, status, priority, created_by, created_at, updated_at)
             VALUES ('t', 'ready', 'medium', 'user', ?1, ?1)",
            [now],
        );
        assert!(r.is_ok());
        let r = conn.execute(
            "INSERT INTO task_note (id, task_id, thread_id, body, author, created_at)
             VALUES ('n-bad2', 1, 'b-1', 'b', 'u', ?1)",
            [now],
        );
        assert!(r.is_err(), "task_note with both parents should fail CHECK");
    }

    /// Regression: the first version of V18 rebuilt the `task` table
    /// via `task_new` + `DROP TABLE task` + rename, which under
    /// `PRAGMA foreign_keys = ON` cascaded and wiped every
    /// `task_effort` row (`task_effort.task_id REFERENCES task(id)
    /// ON DELETE CASCADE`). The fixed migration uses
    /// `ALTER TABLE … DROP COLUMN` instead, which leaves child rows
    /// untouched. This test asserts a `task_effort` row created
    /// AFTER all migrations have run (including V18) coexists with
    /// its parent and the `acceptance_criteria` column is gone.
    #[test]
    fn v18_does_not_cascade_to_task_effort() {
        let db = Database::in_memory();
        let conn = db.conn().unwrap();
        let now = "2026-04-29T00:00:00Z";
        conn.execute(
            "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
             VALUES ('s-1', 'primary', 'a', 'main', 'r', 'r', '/r', ?1, ?1)",
            [now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
             VALUES ('b-1', 's-1', 't', 'active', ?1, ?1)",
            [now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO task (thread_id, title, status, priority, created_by, created_at, updated_at)
             VALUES ('b-1', 't', 'in_progress', 'medium', 'user', ?1, ?1)",
            [now],
        )
        .unwrap();
        let task_id: i64 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO task_effort (id, task_id, thread_id, started_at)
             VALUES ('ef-test', ?1, 'b-1', ?2)",
            (task_id, now),
        )
        .unwrap();

        // task_effort row survives alongside its parent.
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM task_effort WHERE task_id = ?1",
                [task_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "task_effort row must coexist with its parent task");

        // acceptance_criteria column is gone.
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(task)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(
            !cols.iter().any(|c| c == "acceptance_criteria"),
            "task.acceptance_criteria column must be dropped by V18 (cols: {cols:?})"
        );
    }
}
