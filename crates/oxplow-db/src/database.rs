use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::Connection;
use tracing::info;

mod embedded {
    refinery::embed_migrations!("migrations");
}

/// Database-scoped memo cells for expensive read-only lookups whose
/// invalidation the owning store controls.
///
/// It lives on [`Database`] rather than on the store that uses it because the
/// app builds **several store instances over the same `Database`** — `Services`
/// holds one `SqliteFactStore`, `MetricEngine` constructs another, and tests
/// build more from the same `db.clone()`. A per-store cache would let a write
/// through one instance leave another instance's copy stale, which is a
/// correctness bug (a new producer's facts silently missing from reads), not
/// just a missed optimization. Cloning a `Database` shares this, exactly as it
/// already shares the pool.
#[derive(Default)]
pub struct QueryMemo {
    /// `measure_id` → the producers that have emitted facts for it (tsk130).
    producers_for_measure: Mutex<HashMap<i64, Vec<String>>>,
    /// Bumped on every fact write. Read-side stores the generation they queried
    /// under and refuse to cache a result computed across a write — see
    /// [`Self::producers_put`].
    facts_generation: AtomicU64,
}

impl QueryMemo {
    /// A poisoned memo is not a reason to take the process down: it's a cache,
    /// and the worst a poisoned map holds is a value we'd have recomputed.
    fn producers(&self) -> std::sync::MutexGuard<'_, HashMap<i64, Vec<String>>> {
        self.producers_for_measure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Look up a memoized producer list, along with the generation it was read
    /// under — pass that back to [`Self::producers_put`].
    pub(crate) fn producers_get(&self, measure_id: i64) -> (u64, Option<Vec<String>>) {
        let generation = self.facts_generation.load(Ordering::Acquire);
        let hit = self.producers().get(&measure_id).cloned();
        (generation, hit)
    }

    /// Memoize a producer list, but **only if no fact write landed since
    /// `generation`**. Without that check a query that started before a write
    /// and finished after it would install a result missing the new producer,
    /// and nothing would clear it until the *next* write — a metric silently
    /// blind to a producer.
    pub(crate) fn producers_put(&self, measure_id: i64, generation: u64, value: &[String]) {
        if self.facts_generation.load(Ordering::Acquire) != generation {
            return;
        }
        self.producers().insert(measure_id, value.to_vec());
    }

    /// Called after facts are committed: bump the generation (so an in-flight
    /// read declines to cache) and drop what's memoized.
    pub(crate) fn invalidate_facts(&self) {
        self.facts_generation.fetch_add(1, Ordering::AcqRel);
        self.producers().clear();
    }
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
    memo: Arc<QueryMemo>,
    /// Bounds how many DB tasks are dispatched to the blocking pool at once,
    /// sized to the connection pool (tsk131).
    gate: Arc<Semaphore>,
}

/// Per-connection setup, run by the pool for every connection it opens.
///
/// Everything here is connection-local and lock-free. `journal_mode` is
/// deliberately absent: it is a property of the *file*, set once in
/// [`Database::open`], and doing it per connection means contending for
/// an exclusive lock with whatever else is writing (tsk262).
fn init_connection(c: &Connection) -> rusqlite::Result<()> {
    c.pragma_update(None, "foreign_keys", "ON")?;
    c.pragma_update(None, "synchronous", "NORMAL")?;
    // The hot metric store paths use `prepare_cached` (tsk112);
    // rusqlite's default LRU is 16 statements, small enough that the
    // build/read mix would thrash it and re-parse anyway.
    c.set_prepared_statement_cache_capacity(128);
    Ok(())
}

impl Database {
    /// Open (or create) the SQLite file at `path` and apply migrations.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DbInitError> {
        // `journal_mode` is persisted in the file, so set it once here
        // rather than from every pooled connection. It needs an
        // exclusive lock and SQLite does NOT route it through
        // `busy_timeout`, so a connection coming up while migrations
        // hold a write transaction fails instantly — which is what made
        // every fresh project log `ERROR database is locked` (tsk262).
        let setup = Connection::open(path.as_ref()).map_err(DbInitError::Sqlite)?;
        setup
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(DbInitError::Sqlite)?;
        drop(setup);

        let manager =
            SqliteConnectionManager::file(path.as_ref()).with_init(|c| init_connection(c));
        let pool = Pool::builder()
            .max_size(8)
            .build(manager)
            .map_err(DbInitError::Pool)?;

        let mut conn = pool.get().map_err(DbInitError::Pool)?;
        embedded::migrations::runner()
            .run(&mut *conn)
            .map_err(|e| DbInitError::Migration(e.to_string()))?;
        info!("oxplow db opened at {}", path.as_ref().display());

        let permits = pool.max_size() as usize;
        Ok(Self {
            pool: Arc::new(pool),
            memo: Arc::new(QueryMemo::default()),
            gate: Arc::new(Semaphore::new(permits)),
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
        let permits = pool.max_size() as usize;
        Self {
            pool: Arc::new(pool),
            memo: Arc::new(QueryMemo::default()),
            gate: Arc::new(Semaphore::new(permits)),
        }
    }

    /// Take a slot before dispatching DB work to the blocking pool.
    ///
    /// Sized to the connection pool: without it, `Database::call` spawns a
    /// blocking thread per caller, and the metric path fanned out to ~197 of
    /// them against 8 connections — ~189 OS threads (2 MB of stack each) whose
    /// entire job was to block inside `pool.get()` (tsk131). Waiting here
    /// instead makes the queue a cheap async wait. Throughput is unchanged:
    /// only `max_size` tasks could ever hold a connection anyway.
    ///
    /// Safe against deadlock because a permit is only held across the
    /// `spawn_blocking` itself, and the closures are synchronous — they cannot
    /// await another gated call, so permits never nest.
    async fn db_permit(&self) -> Result<OwnedSemaphorePermit, oxplow_domain::DomainError> {
        self.gate
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| oxplow_domain::DomainError::Storage("db gate closed".into()))
    }

    /// The shared memo cells (see [`QueryMemo`]). Scoped to this `Database`,
    /// so every store built over the same handle sees one another's
    /// invalidations.
    pub(crate) fn memo(&self) -> &QueryMemo {
        &self.memo
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
            .map_err(|e| oxplow_domain::DomainError::Storage(format!("pool: {e}")))?;
        f(&conn).map_err(map_sql_err)
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
        let permit = self.db_permit().await?;
        let db = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit; // held for the duration of the DB work
            db.with_conn(f)
        })
        .await
        .map_err(|e| oxplow_domain::DomainError::Storage(format!("db task panicked: {e}")))?
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
        let permit = self.db_permit().await?;
        let db = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut conn = db
                .conn()
                .map_err(|e| oxplow_domain::DomainError::Storage(format!("pool: {e}")))?;
            f(&mut conn)
        })
        .await
        .map_err(|e| oxplow_domain::DomainError::Storage(format!("db task panicked: {e}")))?
    }
}

impl Database {
    /// Run `f` inside a single SQLite transaction, off the async
    /// runtime. THE composition point for multi-write actions: services
    /// compose sync `*_tx(conn, …)` store cores inside one closure so a
    /// user-visible action commits or rolls back as a unit (see
    /// `.context/data-model.md`, "Transactions").
    ///
    /// Owns the `SQLITE_BUSY` retry: a failed attempt rolled back and
    /// left no trace, so re-running the closure is safe by
    /// construction — which is why `f` is `Fn`, not `FnOnce`, and why
    /// retry lives here and nowhere else. Only [`DomainError::Busy`]
    /// retries (bounded, short backoff); every other error — including
    /// `Constraint` — returns immediately after rollback.
    ///
    /// Event-bus emits, page_ref projections, and snapshot requests
    /// belong AFTER this returns, never inside `f`.
    pub async fn transaction<R, F>(&self, f: F) -> Result<R, oxplow_domain::DomainError>
    where
        F: Fn(&rusqlite::Transaction<'_>) -> Result<R, oxplow_domain::DomainError> + Send + 'static,
        R: Send + 'static,
    {
        const MAX_ATTEMPTS: u32 = 3;
        const BACKOFF: [std::time::Duration; 2] = [
            std::time::Duration::from_millis(50),
            std::time::Duration::from_millis(200),
        ];
        let permit = self.db_permit().await?;
        let db = self.clone();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut attempt: u32 = 0;
            loop {
                attempt += 1;
                let mut conn = db
                    .conn()
                    .map_err(|e| oxplow_domain::DomainError::Storage(format!("pool: {e}")))?;
                let tx = conn.transaction().map_err(map_sql_err)?;
                let outcome = f(&tx).and_then(|value| {
                    tx.commit().map_err(map_sql_err)?;
                    Ok(value)
                });
                match outcome {
                    Ok(value) => return Ok(value),
                    Err(err) if err.is_retryable() && attempt < MAX_ATTEMPTS => {
                        // Dropped `tx` already rolled back; safe to rerun.
                        std::thread::sleep(BACKOFF[(attempt - 1) as usize % BACKOFF.len()]);
                    }
                    Err(err) => return Err(err),
                }
            }
        })
        .await
        .map_err(|e| oxplow_domain::DomainError::Storage(format!("db task panicked: {e}")))?
    }
}

/// Classify a `rusqlite::Error` into the typed `DomainError` storage
/// variants so upper layers can implement retry / user-message policy
/// instead of pattern-matching on stringified SQL errors:
/// constraint violations → `Constraint`, `SQLITE_BUSY`/`SQLITE_LOCKED`
/// → `Busy` (retryable), everything else → `Storage`.
/// Normalize an RFC-3339 UTC timestamp string to a **fixed-width** canonical
/// form (`YYYY-MM-DDTHH:MM:SS.ffffffZ`, always 6 fractional digits) for SQLite
/// storage. The `time` crate's RFC-3339 formatter trims trailing fractional
/// zeros (and omits the fraction entirely when it's zero), so a whole-second
/// `…20Z` and a `…20.123Z` at the same second sort in the *wrong* order under
/// SQLite's lexicographic comparison — which breaks the `captured_at` range /
/// `ORDER BY` queries the metric substrate and effort-window overlay rely on.
/// Pinning the fraction to a fixed width makes string order match chronological
/// order. Sub-microsecond precision is truncated (ordering-preserving). A
/// non-UTC / unexpected shape is returned unchanged (all oxplow timestamps are
/// UTC, so this is a safety fallback, not a real path).
pub(crate) fn canonical_ts(rfc3339_utc: &str) -> String {
    let Some(body) = rfc3339_utc.strip_suffix('Z') else {
        return rfc3339_utc.to_string();
    };
    let (datetime, frac) = match body.split_once('.') {
        Some((d, f)) => (d, f),
        None => (body, ""),
    };
    let mut frac6: String = frac
        .chars()
        .take(6)
        .filter(|c| c.is_ascii_digit())
        .collect();
    while frac6.len() < 6 {
        frac6.push('0');
    }
    format!("{datetime}.{frac6}Z")
}

pub(crate) fn map_sql_err(e: rusqlite::Error) -> oxplow_domain::DomainError {
    use rusqlite::ffi::ErrorCode;
    match &e {
        rusqlite::Error::SqliteFailure(f, _) => match f.code {
            ErrorCode::ConstraintViolation => {
                oxplow_domain::DomainError::Constraint(format!("sql: {e}"))
            }
            ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => {
                oxplow_domain::DomainError::Busy(format!("sql: {e}"))
            }
            _ => oxplow_domain::DomainError::Storage(format!("sql: {e}")),
        },
        _ => oxplow_domain::DomainError::Storage(format!("sql: {e}")),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DbInitError {
    #[error("connection pool: {0}")]
    Pool(r2d2::Error),
    #[error("migration: {0}")]
    Migration(String),
    #[error("sqlite: {0}")]
    Sqlite(rusqlite::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bringing a pool connection up must never contend with a writer.
    ///
    /// This is the shape of first boot: r2d2 keeps filling the pool
    /// toward `max_size` in the background while `open` is already
    /// running migrations inside a write transaction. If the per-
    /// connection init needs an exclusive lock, those connections fail —
    /// and `journal_mode` is exactly such an operation, one that SQLite
    /// does *not* route through `busy_timeout`, so it fails instantly
    /// rather than waiting. r2d2's default error handler logged the
    /// result as `ERROR database is locked` on every fresh project,
    /// for something that its own retry then resolved (tsk262).
    #[test]
    fn connection_init_never_contends_with_a_writer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite");
        // A brand-new file is still journal_mode=delete — the state the
        // pool comes up against on first boot.
        let writer = Connection::open(&path).unwrap();
        writer
            .execute_batch("BEGIN IMMEDIATE; CREATE TABLE probe(x);")
            .unwrap();

        let late = Connection::open(&path).unwrap();
        init_connection(&late).expect("per-connection init must not need an exclusive lock");
    }

    /// `journal_mode` moved out of the per-connection init, so pin that
    /// opening a database still leaves it in WAL.
    #[test]
    fn open_leaves_the_database_in_wal_mode() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path().join("test.sqlite")).unwrap();
        let mode: String = db
            .conn()
            .unwrap()
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode.to_lowercase(), "wal");
    }

    /// Provoke real rusqlite errors through a scratch connection and
    /// check `map_sql_err`'s classification — upper layers key retry /
    /// user-message policy off these variants.
    #[test]
    fn map_sql_err_classifies_constraint_busy_and_other() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY, v TEXT UNIQUE)")
            .unwrap();
        conn.execute("INSERT INTO t (v) VALUES ('a')", []).unwrap();
        let dup = conn
            .execute("INSERT INTO t (v) VALUES ('a')", [])
            .unwrap_err();
        assert!(matches!(
            map_sql_err(dup),
            oxplow_domain::DomainError::Constraint(_)
        ));

        let missing_table = conn
            .execute("INSERT INTO nope (v) VALUES (1)", [])
            .unwrap_err();
        assert!(matches!(
            map_sql_err(missing_table),
            oxplow_domain::DomainError::Storage(_)
        ));

        // Busy is hard to provoke deterministically on :memory:; build
        // the ffi error directly.
        let busy = rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
            Some("database is locked".into()),
        );
        let mapped = map_sql_err(busy);
        assert!(matches!(mapped, oxplow_domain::DomainError::Busy(_)));
        assert!(mapped.is_retryable());
    }

    #[test]
    fn canonical_ts_is_fixed_width_and_orders_chronologically() {
        // Variable-width RFC-3339 (the time crate's output) → fixed 6-digit.
        assert_eq!(
            canonical_ts("2023-11-14T22:13:20Z"),
            "2023-11-14T22:13:20.000000Z"
        );
        assert_eq!(
            canonical_ts("2023-11-14T22:13:20.1Z"),
            "2023-11-14T22:13:20.100000Z"
        );
        assert_eq!(
            canonical_ts("2023-11-14T22:13:20.123Z"),
            "2023-11-14T22:13:20.123000Z"
        );
        // Sub-microsecond is truncated (ordering-preserving).
        assert_eq!(
            canonical_ts("2023-11-14T22:13:20.123456789Z"),
            "2023-11-14T22:13:20.123456Z"
        );
        // The headline bug: a whole second and a fraction at the SAME second now
        // sort chronologically (raw strings sort the whole second LAST because
        // 'Z' > '.').
        let whole = canonical_ts("2023-11-14T22:13:20Z");
        let frac = canonical_ts("2023-11-14T22:13:20.123Z");
        assert!(whole < frac, "{whole} should sort before {frac}");
        // Non-UTC / unexpected shape is passed through untouched.
        assert_eq!(canonical_ts("not-a-timestamp"), "not-a-timestamp");
    }

    #[tokio::test]
    async fn transaction_commits_all_writes() {
        let db = Database::in_memory();
        db.transaction(|tx| {
            tx.execute_batch("CREATE TABLE t (v TEXT)")
                .map_err(map_sql_err)?;
            tx.execute("INSERT INTO t (v) VALUES ('a')", [])
                .map_err(map_sql_err)?;
            tx.execute("INSERT INTO t (v) VALUES ('b')", [])
                .map_err(map_sql_err)?;
            Ok(())
        })
        .await
        .unwrap();
        let n: i64 = db
            .call(|conn| conn.query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(n, 2);
    }

    #[tokio::test]
    async fn transaction_rolls_back_on_closure_error() {
        let db = Database::in_memory();
        db.call(|conn| conn.execute_batch("CREATE TABLE t (v TEXT)"))
            .await
            .unwrap();
        let err = db
            .transaction(|tx| {
                tx.execute("INSERT INTO t (v) VALUES ('a')", [])
                    .map_err(map_sql_err)?;
                Err::<(), _>(oxplow_domain::DomainError::Invariant(
                    "midway failure".into(),
                ))
            })
            .await
            .unwrap_err();
        assert!(matches!(err, oxplow_domain::DomainError::Invariant(_)));
        // The pre-error insert must not survive.
        let n: i64 = db
            .call(|conn| conn.query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(n, 0);
    }

    #[tokio::test]
    async fn transaction_retries_busy_but_not_constraint() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        let db = Database::in_memory();
        // Busy on the first two attempts, success on the third — the
        // bounded retry must absorb the transient contention.
        let calls = Arc::new(AtomicU32::new(0));
        let seen = calls.clone();
        let out = db
            .transaction(move |_tx| {
                if seen.fetch_add(1, Ordering::SeqCst) < 2 {
                    Err(oxplow_domain::DomainError::Busy("locked".into()))
                } else {
                    Ok(42)
                }
            })
            .await
            .unwrap();
        assert_eq!(out, 42);
        assert_eq!(calls.load(Ordering::SeqCst), 3);

        // Constraint is deterministic — exactly one attempt.
        let calls = Arc::new(AtomicU32::new(0));
        let seen = calls.clone();
        let err = db
            .transaction(move |_tx| {
                seen.fetch_add(1, Ordering::SeqCst);
                Err::<(), _>(oxplow_domain::DomainError::Constraint("dup".into()))
            })
            .await
            .unwrap_err();
        assert!(matches!(err, oxplow_domain::DomainError::Constraint(_)));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

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
            // V43 — the durable atomic fact layer (the sole metric substrate
            // since T-E3 dropped the V38 cluster, V49). The `subject` hierarchy
            // table was dropped in V52 (tsk15) — never read/written.
            "measure",
            "dimension",
            "metric_capture",
            "fact",
            // V44 — the metric SPEC layer.
            "metric_spec",
            // V62 — the aggregate cube: the materialized fold (tsk96), its
            // durable live state, and the watermark — both keyed per
            // (measure, stream, branch) since V63 (tsk97). V66 adds the
            // global invalidation epoch that fences in-flight builds.
            "metric_cube",
            "metric_live_fact",
            "metric_cube_state",
            "metric_cube_epoch",
        ];
        // `effort_observation` was dropped in V39 (tsk215); the V38
        // `metric_*` cluster was dropped in V49 (T-E3, tsk50) — assert gone.
        let expected_absent = [
            "hook_event",
            "agent_status",
            "effort_observation",
            "metric_definition",
            "metric_dimension",
            "metric_subject",
            "metric_run",
            "metric_sample",
            "metric_finding",
        ];
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
             VALUES (1, 'primary', 'a', 'main', 'refs/heads/main', 'main', '/r', ?1, ?1)",
            [now],
        ).unwrap();
        let result = conn.execute(
            "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
             VALUES (2, 'primary', 'b', 'main', 'refs/heads/main', 'main', '/r', ?1, ?1)",
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
             VALUES (1, 'primary', 'a', 'main', 'refs/heads/main', 'main', '/r', ?1, ?1)",
            [now],
        ).unwrap();
        conn.execute(
            "INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
             VALUES (1, 1, 'a', 'active', ?1, ?1)",
            [now],
        )
        .unwrap();
        let result = conn.execute(
            "INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
             VALUES (2, 1, 'b', 'active', ?1, ?1)",
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
             VALUES (1, 999, 't', 'queued', '2026-01-01', '2026-01-01')",
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
             VALUES (1, 'primary', 'a', 'main', 'r', 'r', '/r', ?1, ?1)",
            [now],
        ).unwrap();
        conn.execute(
            "INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
             VALUES (1, 1, 't', 'active', ?1, ?1)",
            [now],
        )
        .unwrap();
        // Both null — must fail.
        let r = conn.execute(
            "INSERT INTO task_note (body, author, created_at) VALUES ('b', 'u', ?1)",
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
            "INSERT INTO task_note (task_id, thread_id, body, author, created_at)
             VALUES (1, 1, 'b', 'u', ?1)",
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
             VALUES (1, 'primary', 'a', 'main', 'r', 'r', '/r', ?1, ?1)",
            [now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
             VALUES (1, 1, 't', 'active', ?1, ?1)",
            [now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO task (thread_id, title, status, priority, created_by, created_at, updated_at)
             VALUES (1, 't', 'in_progress', 'medium', 'user', ?1, ?1)",
            [now],
        )
        .unwrap();
        let task_id: i64 = conn.last_insert_rowid();
        conn.execute(
            "INSERT INTO task_effort (task_id, thread_id, started_at)
             VALUES (?1, 1, ?2)",
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
