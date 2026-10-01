use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use oxplow_domain::{DomainError, Timestamp};
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
    /// Which tables commits touched, published after each call (P4.6).
    changes: Arc<crate::changes::Changes>,
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
    /// Open an existing project database **read-only**, without migrating
    /// or writing anything — for a tool (the `oxplow plugin check` CLI)
    /// that may be a different oxplow version than the app that owns the
    /// file. Refuses a file whose schema version differs from this build's
    /// (its views may not match what this build would query).
    pub fn open_read_only(path: impl AsRef<Path>) -> Result<Self, DbInitError> {
        use rusqlite::OpenFlags;
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let probe =
            Connection::open_with_flags(path.as_ref(), flags).map_err(DbInitError::Sqlite)?;
        let have: Option<i64> = probe
            .query_row(
                "SELECT max(version) FROM refinery_schema_history",
                [],
                |r| r.get(0),
            )
            .map_err(DbInitError::Sqlite)?;
        drop(probe);
        let want = embedded::migrations::runner()
            .get_migrations()
            .iter()
            .map(|m| i64::from(m.version()))
            .max();
        if have != want {
            return Err(DbInitError::Migration(format!(
                "schema version {} but this oxplow expects {}; open the project in a matching oxplow first",
                have.map_or("none".into(), |v| v.to_string()),
                want.map_or("none".into(), |v| v.to_string()),
            )));
        }
        let changes = Arc::new(crate::changes::Changes::default());
        let hooks = changes.clone();
        let manager = SqliteConnectionManager::file(path.as_ref())
            .with_flags(flags)
            .with_init(move |c| {
                c.pragma_update(None, "foreign_keys", "ON")?;
                c.busy_timeout(std::time::Duration::from_secs(5))?;
                hooks.install(c)
            });
        let pool = Pool::builder()
            .max_size(2)
            .build(manager)
            .map_err(DbInitError::Pool)?;
        let permits = pool.max_size() as usize;
        Ok(Self {
            pool: Arc::new(pool),
            memo: Arc::new(QueryMemo::default()),
            gate: Arc::new(Semaphore::new(permits)),
            changes,
        })
    }

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

        let changes = Arc::new(crate::changes::Changes::default());
        let hooks = changes.clone();
        let manager = SqliteConnectionManager::file(path.as_ref()).with_init(move |c| {
            init_connection(c)?;
            hooks.install(c)
        });
        let pool = Pool::builder()
            .max_size(8)
            .build(manager)
            .map_err(DbInitError::Pool)?;

        let mut conn = pool.get().map_err(DbInitError::Pool)?;
        migrate_and_compile(&mut conn)?;
        info!("oxplow db opened at {}", path.as_ref().display());

        let permits = pool.max_size() as usize;
        Ok(Self {
            pool: Arc::new(pool),
            memo: Arc::new(QueryMemo::default()),
            gate: Arc::new(Semaphore::new(permits)),
            changes,
        })
    }

    /// In-memory DB for tests. Each call returns a fresh DB.
    ///
    /// Public so other crates' tests can build a Services graph
    /// without needing a tempfile.
    pub fn in_memory() -> Self {
        let changes = Arc::new(crate::changes::Changes::default());
        let hooks = changes.clone();
        let manager = SqliteConnectionManager::memory().with_init(move |c| {
            c.pragma_update(None, "foreign_keys", "ON")?;
            hooks.install(c)
        });
        let pool = Pool::builder()
            .max_size(1)
            .build(manager)
            .expect("in-memory sqlite pool builds");
        let mut conn = pool.get().expect("in-memory sqlite connection");
        migrate_and_compile(&mut conn).expect("in-memory migrations and models");
        let permits = pool.max_size() as usize;
        Self {
            pool: Arc::new(pool),
            memo: Arc::new(QueryMemo::default()),
            gate: Arc::new(Semaphore::new(permits)),
            changes,
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
        let out = tokio::task::spawn_blocking(move || {
            let _permit = permit; // held for the duration of the DB work
            db.with_conn(f)
        })
        .await
        .map_err(|e| oxplow_domain::DomainError::Storage(format!("db task panicked: {e}")))?;
        self.changes.flush();
        out
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
        let out = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut conn = db
                .conn()
                .map_err(|e| oxplow_domain::DomainError::Storage(format!("pool: {e}")))?;
            f(&mut conn)
        })
        .await
        .map_err(|e| oxplow_domain::DomainError::Storage(format!("db task panicked: {e}")))?;
        self.changes.flush();
        out
    }

    /// Hear which tables commits touched: each message is the tables one or
    /// more calls committed writes to, sent after they committed (P4.6).
    /// A lagging receiver misses messages; treat a lag as "anything may
    /// have changed".
    pub fn subscribe_changes(
        &self,
    ) -> tokio::sync::broadcast::Receiver<crate::changes::TablesChanged> {
        self.changes.subscribe()
    }
}

impl Database {
    /// Compile the enabled extensions' models over the core ones
    /// ([`crate::models::compile_extensions`]): each extension's errors.
    pub async fn compile_extension_models(
        &self,
        extensions: Vec<crate::models::ExtensionModels>,
    ) -> Result<std::collections::BTreeMap<String, Vec<String>>, oxplow_domain::DomainError> {
        self.call_mut(move |conn| crate::models::compile_extensions(conn, &extensions))
            .await
    }

    /// Check the extensions' models without publishing them — works on a
    /// read-only database ([`crate::models::check_extensions`]).
    pub async fn check_extension_models(
        &self,
        extensions: Vec<crate::models::ExtensionModels>,
    ) -> Result<std::collections::BTreeMap<String, Vec<String>>, oxplow_domain::DomainError> {
        self.read(move |tx| crate::models::check_extensions(tx, &extensions))
            .await
    }

    /// Run a read-only closure off the async runtime, in a DEFERRED
    /// transaction: one consistent snapshot across its statements, and no
    /// write lock (a [`Self::transaction`] begins IMMEDIATE). Always rolled
    /// back, so a stray write in `f` never lands.
    pub async fn read<R, F>(&self, f: F) -> Result<R, oxplow_domain::DomainError>
    where
        F: FnOnce(&rusqlite::Transaction<'_>) -> Result<R, oxplow_domain::DomainError>
            + Send
            + 'static,
        R: Send + 'static,
    {
        self.call_mut(move |conn| {
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
                .map_err(map_sql_err)?;
            let out = f(&tx)?;
            tx.rollback().map_err(map_sql_err)?;
            Ok(out)
        })
        .await
    }

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
        self.write_transaction(f, true).await
    }

    /// [`Self::transaction`], always rolled back: what `f` would do, with
    /// the write lock and the busy retry of a real run, and nothing kept.
    /// A command's dry run (a proposal's preview) runs its handler here.
    pub async fn rehearse<R, F>(&self, f: F) -> Result<R, oxplow_domain::DomainError>
    where
        F: Fn(&rusqlite::Transaction<'_>) -> Result<R, oxplow_domain::DomainError> + Send + 'static,
        R: Send + 'static,
    {
        self.write_transaction(f, false).await
    }

    async fn write_transaction<R, F>(
        &self,
        f: F,
        commit: bool,
    ) -> Result<R, oxplow_domain::DomainError>
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
        let out = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut attempt: u32 = 0;
            loop {
                attempt += 1;
                let mut conn = db
                    .conn()
                    .map_err(|e| oxplow_domain::DomainError::Storage(format!("pool: {e}")))?;
                // IMMEDIATE: take the write lock at BEGIN (waiting under
                // `busy_timeout`). A deferred read-then-write transaction
                // fails with SQLITE_BUSY_SNAPSHOT when another commits
                // between its read and its first write, which no wait fixes.
                let tx = conn
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    .map_err(map_sql_err)?;
                let outcome = f(&tx).and_then(|value| {
                    if commit {
                        tx.commit().map_err(map_sql_err)?;
                    } else {
                        tx.rollback().map_err(map_sql_err)?;
                    }
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
        .map_err(|e| oxplow_domain::DomainError::Storage(format!("db task panicked: {e}")));
        self.changes.flush();
        out?
    }
}

/// Classify a `rusqlite::Error` into the typed `DomainError` storage
/// variants so upper layers can implement retry / user-message policy
/// instead of pattern-matching on stringified SQL errors:
/// constraint violations → `Constraint`, `SQLITE_BUSY`/`SQLITE_LOCKED`
/// → `Busy` (retryable), everything else → `Storage`.
/// A `Timestamp` as the TEXT SQLite stores: the fixed-width RFC 3339 form
/// (`YYYY-MM-DDTHH:MM:SS.ffffffZ`, 27 chars) that `Timestamp` itself
/// produces, so lexicographic `ORDER BY` / `BETWEEN` on a timestamp column
/// is chronological. Every store goes through this one helper (tsk387);
/// V95 normalized the rows written before the serializer was fixed.
pub(crate) fn ts_to_string(ts: Timestamp) -> String {
    ts.to_text()
}

/// The inverse of [`ts_to_string`]; accepts any RFC 3339 text (rows from
/// before V95, other producers).
pub(crate) fn string_to_ts(s: &str) -> Result<Timestamp, DomainError> {
    Timestamp::parse(s).map_err(|e| DomainError::Invalid(format!("bad timestamp `{s}`: {e}")))
}

pub fn map_sql_err(e: rusqlite::Error) -> oxplow_domain::DomainError {
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
    #[error("models: {0}")]
    Models(String),
}

/// Migrate `conn` up to `version` only — a migration test's "before".
#[cfg(test)]
pub(crate) fn migrate_to_for_tests(conn: &mut Connection, version: i32) {
    embedded::migrations::runner()
        .set_target(refinery::Target::Version(version))
        .run(conn)
        .expect("migrations run");
}

/// Bring a database to this build: drop the model views, apply the
/// migrations, then compile the models (P4.2). The views are recreated at
/// every open, so a migration never works around one — and never creates
/// one: a published view is a model file.
pub(crate) fn migrate_and_compile(conn: &mut Connection) -> Result<(), DbInitError> {
    crate::models::drop_all(conn).map_err(|e| DbInitError::Models(e.to_string()))?;
    embedded::migrations::runner()
        .run(conn)
        .map_err(|e| DbInitError::Migration(e.to_string()))?;
    crate::models::compile_core(conn).map_err(|e| DbInitError::Models(e.to_string()))
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
    fn ts_helpers_are_fixed_width_and_accept_old_rows() {
        let half = Timestamp::from_unix_ms(1_700_000_000_500);
        let text = ts_to_string(half);
        assert_eq!(text, "2023-11-14T22:13:20.500000Z");
        assert_eq!(text.len(), Timestamp::TEXT_LEN);
        // Rows written before V95 (trimmed) still read back.
        assert_eq!(string_to_ts("2023-11-14T22:13:20.5Z").unwrap(), half);
        assert_eq!(string_to_ts(&text).unwrap(), half);
        let err = string_to_ts("nope").unwrap_err();
        assert!(matches!(err, DomainError::Invalid(_)), "{err}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_read_then_write_transactions_all_commit() {
        // Each transaction reads, pauses, then writes — the shape of the
        // hook ingest. Deferred BEGIN would let a reader's snapshot go stale
        // under another's commit (SQLITE_BUSY_SNAPSHOT, which no busy wait
        // can fix); IMMEDIATE takes the write lock up front and waits.
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path().join("c.db")).unwrap();
        db.transaction(|tx| {
            tx.execute_batch("CREATE TABLE n (v INTEGER NOT NULL)")
                .map_err(map_sql_err)
        })
        .await
        .unwrap();
        let tasks: Vec<_> = (0..16)
            .map(|_| {
                let db = db.clone();
                tokio::spawn(async move {
                    db.transaction(|tx| {
                        let count: i64 = tx
                            .query_row("SELECT COUNT(*) FROM n", [], |r| r.get(0))
                            .map_err(map_sql_err)?;
                        std::thread::sleep(std::time::Duration::from_millis(5));
                        tx.execute("INSERT INTO n (v) VALUES (?1)", [count])
                            .map_err(map_sql_err)?;
                        Ok(())
                    })
                    .await
                })
            })
            .collect();
        for t in tasks {
            t.await.unwrap().unwrap();
        }
        let values: Vec<i64> = db
            .transaction(|tx| {
                let mut s = tx
                    .prepare("SELECT v FROM n ORDER BY v")
                    .map_err(map_sql_err)?;
                let r = s
                    .query_map([], |r| r.get(0))
                    .map_err(map_sql_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(map_sql_err)?;
                Ok(r)
            })
            .await
            .unwrap();
        // Serialized: every transaction saw the ones before it.
        assert_eq!(values, (0..16).collect::<Vec<_>>());
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
            "effort",
            "effort_file",
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
            // V93 — the event log as an outbox, its checkpoints and dead
            // letters, and the command audit (tsk406).
            "event_log",
            "event_consumer_checkpoint",
            "event_dead_letter",
            "command_audit",
        ];
        // `effort_observation` was dropped in V39 (tsk215); the V38
        // `metric_*` cluster was dropped in V49 (T-E3, tsk50) — assert gone.
        let expected_absent = [
            // V93 dropped the dead per-task audit table (tsk406).
            "task_event",
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

    #[test]
    fn open_read_only_neither_migrates_nor_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("local.sqlite");
        // Not a database, or an older schema: refused, file untouched.
        std::fs::write(&path, "junk").unwrap();
        assert!(Database::open_read_only(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"junk");
        std::fs::remove_file(&path).unwrap();
        {
            let mut conn = rusqlite::Connection::open(&path).unwrap();
            // As `open` does: a migration's WAL pragma can't run inside
            // the migration transaction on a file database.
            conn.pragma_update(None, "journal_mode", "WAL").unwrap();
            embedded::migrations::runner()
                .set_target(refinery::Target::Version(94))
                .run(&mut conn)
                .unwrap();
        }
        let err = match Database::open_read_only(&path) {
            Ok(_) => panic!("an older schema must be refused"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("schema version 94"), "{err}");
        // A current database opens and can be read, but not written.
        std::fs::remove_file(&path).unwrap();
        drop(Database::open(&path).unwrap());
        let db = Database::open_read_only(&path).unwrap();
        let conn = db.conn().unwrap();
        let n: i64 = conn
            .query_row("SELECT count(*) FROM v_task", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        assert!(conn.execute("DELETE FROM task", []).is_err());
    }

    /// V95 pads every trimmed timestamp to the fixed-width form. Migrate to
    /// V94, write the three shapes the `time` crate used to emit, finish
    /// migrating, and check they now sort chronologically.
    #[test]
    fn v95_normalizes_trimmed_timestamps_so_text_order_is_chronological() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        embedded::migrations::runner()
            .set_target(refinery::Target::Version(94))
            .run(&mut conn)
            .unwrap();
        // Chronological: whole second < .5 < .500001 — but the trimmed text
        // sorts them .500001 < .5 < whole.
        for (id, at) in [
            (1, "2023-11-14T22:13:20Z"),
            (2, "2023-11-14T22:13:20.5Z"),
            (3, "2023-11-14T22:13:20.500001Z"),
        ] {
            conn.execute(
                "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                 VALUES (?1, 'worktree', 's', 'b', 'r', 'r', '/r', ?2, ?2)",
                rusqlite::params![id, at],
            )
            .unwrap();
        }
        let before: Vec<i64> = conn
            .prepare("SELECT id FROM streams ORDER BY created_at")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(before, vec![3, 2, 1], "the pre-V95 order is inverted");
        migrate_and_compile(&mut conn).unwrap();
        let after: Vec<(i64, String)> = conn
            .prepare("SELECT id, created_at FROM streams ORDER BY created_at")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            after,
            vec![
                (1, "2023-11-14T22:13:20.000000Z".to_string()),
                (2, "2023-11-14T22:13:20.500000Z".to_string()),
                (3, "2023-11-14T22:13:20.500001Z".to_string()),
            ]
        );
        for (_, at) in &after {
            assert_eq!(string_to_ts(at).unwrap().to_text(), *at);
        }
    }

    /// V97 backfills one `legacy` op per existing snapshot, each pointing
    /// at the previous snapshot of its own stream.
    #[test]
    fn v97_backfills_legacy_ops_with_per_stream_parents() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        embedded::migrations::runner()
            .set_target(refinery::Target::Version(96))
            .run(&mut conn)
            .unwrap();
        conn.execute_batch(
            "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
               VALUES (1, 'primary', 'a', 'main', 'r', 'r', '/a', '2026-01-01T00:00:00.000000Z', '2026-01-01T00:00:00.000000Z'),
                      (2, 'worktree', 'b', 'b', 'r', 'r', '/b', '2026-01-01T00:00:00.000000Z', '2026-01-01T00:00:00.000000Z');
             INSERT INTO snapshot (id, stream_id, created_at) VALUES
               (1, 1, '2026-01-01T00:00:01.000000Z'),
               (2, 2, '2026-01-01T00:00:02.000000Z'),
               (3, 1, '2026-01-01T00:00:03.000000Z');
             INSERT INTO file_snapshot (stream_id, path, blob_hash, size_bytes, captured_at, storage, snapshot_id)
               VALUES (1, 'a', 'h', 1, '2026-01-01T00:00:01.000000Z', 'oxplow', 1),
                      (1, 'b', 'h', 1, '2026-01-01T00:00:01.000000Z', 'oxplow', 1);",
        )
        .unwrap();
        migrate_and_compile(&mut conn).unwrap();
        let ops: Vec<(i64, Option<i64>, String, i64)> = conn
            .prepare("SELECT snapshot_id, parent_snapshot_id, trigger, file_count FROM snapshot_op ORDER BY seq")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            ops,
            vec![
                (1, None, "legacy".into(), 2),
                (2, None, "legacy".into(), 0),
                (3, Some(1), "legacy".into(), 0),
            ]
        );
    }

    /// V95 must name every TEXT timestamp column the schema had at V94; a
    /// column it missed would keep its trimmed rows sorting wrong against
    /// the fixed-width ones written since. Columns added after V95 are born
    /// fixed-width, so the check reads the schema AS OF V94.
    #[test]
    fn v95_covers_every_timestamp_column_in_the_schema() {
        const V95: &str = include_str!("../migrations/V95__fixed_width_timestamps.sql");
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        embedded::migrations::runner()
            .set_target(refinery::Target::Version(94))
            .run(&mut conn)
            .unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT m.name, p.name FROM sqlite_master m JOIN pragma_table_info(m.name) p
                 WHERE m.type = 'table' AND m.name NOT LIKE 'sqlite_%' AND m.name NOT LIKE 'refinery_%'
                   AND (p.name LIKE '%_at' OR p.name = 'at')
                   AND upper(p.type) LIKE 'TEXT%'
                 ORDER BY 1, 2",
            )
            .unwrap();
        let columns: Vec<(String, String)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(columns.len() >= 60, "{}", columns.len());
        let missing: Vec<String> = columns
            .iter()
            .filter(|(t, c)| !V95.contains(&format!("UPDATE {t} SET {c} =")))
            .map(|(t, c)| format!("{t}.{c}"))
            .collect();
        assert!(missing.is_empty(), "V95 does not normalize: {missing:?}");
    }

    /// V90 widens `threads.agent` with a column swap. A table rebuild
    /// would cascade-delete every child row; the swap must not. Migrate to
    /// V89, add a thread with a task and a turn, then finish migrating.
    #[test]
    fn v90_keeps_thread_children_and_accepts_acp() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        embedded::migrations::runner()
            .set_target(refinery::Target::Version(89))
            .run(&mut conn)
            .unwrap();
        let now = "2026-09-28T00:00:00Z";
        conn.execute_batch(&format!(
            "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
               VALUES (1, 'primary', 'a', 'main', 'r', 'r', '/r', '{now}', '{now}');
             INSERT INTO threads (id, stream_id, title, status, agent, created_at, updated_at)
               VALUES (1, 1, 't', 'active', 'codex', '{now}', '{now}');
             INSERT INTO task (id, thread_id, title, status, priority, created_by, created_at, updated_at)
               VALUES (1, 1, 't', 'in_progress', 'medium', 'user', '{now}', '{now}');"
        ))
        .unwrap();
        migrate_and_compile(&mut conn).unwrap();
        let (agent, acp): (String, Option<String>) = conn
            .query_row(
                "SELECT agent, acp_agent FROM threads WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((agent.as_str(), acp), ("codex", None));
        let tasks: i64 = conn
            .query_row("SELECT count(*) FROM task WHERE thread_id = 1", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(tasks, 1, "the thread's task survived the column swap");
        conn.execute(
            &format!("INSERT INTO threads (id, stream_id, title, status, agent, acp_agent, created_at, updated_at)
               VALUES (2, 1, 'g', 'queued', 'acp', 'gemini', '{now}', '{now}')"),
            [],
        )
        .unwrap();
        let view: String = conn
            .query_row("SELECT acp_agent FROM v_thread WHERE id = 2", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(view, "gemini");
        assert!(conn
            .execute(
                &format!("INSERT INTO threads (id, stream_id, title, status, agent, created_at, updated_at)
                   VALUES (3, 1, 'x', 'queued', 'emacs', '{now}', '{now}')"),
                [],
            )
            .is_err());
    }

    /// V100 (tsk427) turns `task_effort` into `effort` in place. Every
    /// child of an effort — CASCADE and SET NULL alike — must survive
    /// (a DROP TABLE on a `foreign_keys=ON` connection would have wiped
    /// or orphaned them, the V18 incident), the task FK becomes a
    /// `work_item` ref, and the two dead tables go.
    #[test]
    fn v100_renames_effort_in_place_and_keeps_every_child() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        embedded::migrations::runner()
            .set_target(refinery::Target::Version(99))
            .run(&mut conn)
            .unwrap();
        let now = "2026-09-29T00:00:00.000000Z";
        conn.execute_batch(&format!(
            "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
               VALUES (1, 'primary', 'a', 'main', 'r', 'r', '/r', '{now}', '{now}');
             INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
               VALUES (1, 1, 't', 'active', '{now}', '{now}');
             INSERT INTO task (id, thread_id, title, status, priority, created_by, created_at, updated_at)
               VALUES (1, 1, 't', 'done', 'medium', 'user', '{now}', '{now}');
             INSERT INTO snapshot (id, stream_id, created_at) VALUES (1, 1, '{now}'), (2, 1, '{now}');
             INSERT INTO task_effort (id, task_id, thread_id, started_at, ended_at, start_snapshot_id, end_snapshot_id, summary)
               VALUES (1, 1, 1, '{now}', '{now}', 1, 2, 'did it');
             INSERT INTO task_effort_file (effort_id, path, change_kind, local_snapshot_id)
               VALUES (1, 'a.rs', 'updated', 2);
             INSERT INTO task_effort_turn (effort_id, turn_id)
               SELECT 1, id FROM agent_turn WHERE 0;
             INSERT INTO effort_acknowledged_path (effort_id, path) VALUES (1, 'b.rs');
             INSERT INTO effort_attribution (effort_id, kind, ref, state, recorded_at)
               VALUES (1, 'file', 'c.rs', 'claimed', '{now}');
             INSERT INTO effort_metric_delta (effort_id, key, title, direction, kind, agg, current, changed, sample_count, refreshed_at)
               VALUES (1, 'k', 'K', 'up', 'gauge', 'level', 1.0, 0, 1, '{now}');
             INSERT INTO effort_observation_row (effort_id, seq, kind, provenance, source, created_at)
               VALUES (1, 1, 'test_run', 'observed', 's', '{now}');
             INSERT INTO effort_unattributed_file (effort_id, path, recorded_at) VALUES (1, 'd.rs', '{now}');
             INSERT INTO agent_nudge (thread_id, effort_id, kind, message, created_at)
               VALUES (1, 1, 'k', 'm', '{now}');
             INSERT INTO agent_token_usage (stream_id, thread_id, effort_id, session_id, agent_kind, provenance, recorded_at)
               VALUES (1, 1, 1, 's', 'claude', 'observed', '{now}');
             INSERT INTO agent_tool_call (thread_id, effort_id, tool, at) VALUES (1, 1, 'Edit', '{now}');
             INSERT INTO claim (thread_id, task_id, effort_id, statement, kind, created_at)
               VALUES (1, 1, 1, 's', 'other', '{now}');
             INSERT INTO decision (thread_id, task_id, effort_id, question, choice, confidence, created_at)
               VALUES (1, 1, 1, 'q', 'c', 'high', '{now}');
             INSERT INTO metric_capture (stream_id, thread_id, effort_id, producer, provenance, source, captured_at)
               VALUES (1, 1, 1, 'p', 'observed', 's', '{now}');
             INSERT INTO snapshot_op (stream_id, snapshot_id, trigger, thread_id, effort_id, at, elapsed_ms, file_count)
               VALUES (1, 2, 'effort_end', 1, 1, '{now}', 1, 1);"
        ))
        .unwrap();

        migrate_and_compile(&mut conn).unwrap();

        let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        for child in [
            "effort_file",
            "effort_acknowledged_path",
            "effort_attribution",
            "effort_metric_delta",
            "effort_observation_row",
            "effort_unattributed_file",
            "agent_nudge",
            "agent_token_usage",
            "agent_tool_call",
            "claim",
            "decision",
            "metric_capture",
            "snapshot_op",
        ] {
            assert_eq!(
                count(&format!("SELECT count(*) FROM {child} WHERE effort_id = 1")),
                1,
                "{child} kept its effort"
            );
        }
        let (work_item, summary, end): (String, String, i64) = conn
            .query_row(
                "SELECT work_item, summary, end_snapshot_id FROM effort WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (work_item.as_str(), summary.as_str(), end),
            ("work_item:oxplow:tsk1", "did it", 2)
        );
        assert_eq!(
            count("SELECT count(*) FROM pragma_table_info('effort') WHERE name = 'task_id'"),
            0
        );
        assert_eq!(
            count(
                "SELECT count(*) FROM sqlite_master WHERE name IN
                   ('task_effort', 'task_effort_file', 'task_effort_turn', 'task_commit')"
            ),
            0
        );
        assert_eq!(count("SELECT task_id FROM v_effort WHERE id = 1"), 1);
        assert_eq!(
            count("SELECT task_id FROM v_effort_file WHERE effort_id = 1"),
            1
        );

        // Another provider's work item: no derived task.
        conn.execute(
            &format!(
                "INSERT INTO effort (id, work_item, thread_id, started_at)
                   VALUES (2, 'work_item:linear:ENG-1', 1, '{now}')"
            ),
            [],
        )
        .unwrap();
        let foreign: Option<i64> = conn
            .query_row("SELECT task_id FROM v_effort WHERE id = 2", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(foreign, None);
        // One open effort per work item.
        assert!(conn
            .execute(
                &format!(
                    "INSERT INTO effort (work_item, thread_id, started_at)
                       VALUES ('work_item:linear:ENG-1', 1, '{now}')"
                ),
                [],
            )
            .is_err());
    }

    /// P4.2 (tsk487): the core models reproduce, column for column and
    /// type for type, every view the migrations made before models existed
    /// (V104) — the sweep moved them without changing a contract. A model
    /// past version 1 has moved on, so it is compared no more.
    #[test]
    fn the_core_models_reproduce_the_views_the_migrations_made() {
        let mut old = rusqlite::Connection::open_in_memory().unwrap();
        embedded::migrations::runner()
            .set_target(refinery::Target::Version(104))
            .run(&mut old)
            .unwrap();
        let mut new = rusqlite::Connection::open_in_memory().unwrap();
        migrate_and_compile(&mut new).unwrap();
        let views: Vec<String> = old
            .prepare("SELECT name FROM sqlite_master WHERE type = 'view' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(views.len(), 49);
        for view in views {
            let version: i64 = new
                .query_row("SELECT version FROM model WHERE view = ?1", [&view], |r| {
                    r.get(0)
                })
                .unwrap_or_else(|_| panic!("{view} is not a model"));
            if version > 1 {
                continue;
            }
            assert_eq!(
                crate::models::view_columns(&new, &view).unwrap(),
                crate::models::view_columns(&old, &view).unwrap(),
                "{view}"
            );
        }
    }

    /// tsk561: before V115, archiving a done task cleared `completed_at`,
    /// so V115's backfill called it `canceled`. V122 restores `done` from
    /// the event log: the task's last archive came from `done`; its
    /// completion is the `done` transition before it.
    #[test]
    fn v122_restores_done_then_archived_tasks_from_the_event_log() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        embedded::migrations::runner()
            .set_target(refinery::Target::Version(121))
            .run(&mut conn)
            .unwrap();
        let t = |n: u32| format!("2026-01-0{n}T00:00:00.000000Z");
        let mut sql = String::new();
        for id in 1..=4 {
            sql += &format!(
                "INSERT INTO task (id, title, status, priority, created_by, created_at, updated_at)
                   VALUES ({id}, 't{id}', 'archived', 'medium', 'agent', '{a}', '{a}');
                 INSERT INTO work_item (ref, provider, title, state, native_state, native,
                                        created_at, updated_at)
                   VALUES ('work_item:oxplow:tsk{id}', 'oxplow', 't{id}', 'canceled', 'archived',
                           json_object('priority', 'medium', 'completed_at', NULL), '{a}', '{a}');",
                a = t(1)
            );
        }
        // tsk1: done, then archived. tsk2: archived from ready. tsk3: done,
        // reopened, then archived. tsk4: no history.
        let moves = [
            (1, "in_progress", "done", 2),
            (1, "done", "archived", 3),
            (2, "ready", "archived", 3),
            (3, "in_progress", "done", 2),
            (3, "done", "ready", 3),
            (3, "ready", "archived", 4),
        ];
        for (i, (task, from, to, day)) in moves.iter().enumerate() {
            sql += &format!(
                "INSERT INTO event_log (id, type, v, at, source, subject, payload)
                   VALUES ('e{i}', 'work_item.transitioned', 1, '{at}', 'test', '[]',
                           json_object('work_item', 'work_item:oxplow:tsk{task}',
                                       'from', '{from}', 'to', '{to}'));",
                at = t(*day)
            );
        }
        conn.execute_batch(&sql).unwrap();

        embedded::migrations::runner()
            .set_target(refinery::Target::Version(122))
            .run(&mut conn)
            .unwrap();
        let rows: Vec<(i64, Option<String>, String, Option<String>)> = conn
            .prepare(
                "SELECT t.id, t.completed_at, w.state, json_extract(w.native, '$.completed_at')
                 FROM task t JOIN work_item w ON w.ref = 'work_item:oxplow:tsk' || t.id
                 ORDER BY t.id",
            )
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            rows,
            vec![
                (1, Some(t(2)), "done".into(), Some(t(2))),
                (2, None, "canceled".into(), None),
                (3, None, "canceled".into(), None),
                (4, None, "canceled".into(), None),
            ]
        );
    }

    /// tsk571: V123 keeps each symbol's name position as its extent start
    /// until it restates, and numbers existing duplicate refs so they can
    /// be unique.
    #[test]
    fn v123_numbers_duplicate_symbol_refs_and_seeds_the_extent() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        embedded::migrations::runner()
            .set_target(refinery::Target::Version(122))
            .run(&mut conn)
            .unwrap();
        conn.execute_batch(
            "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source,
                                  worktree_path, created_at, updated_at)
               VALUES (1, 'primary', 's', 'main', 'r', 'r', '/r', 'now', 'now');
             INSERT INTO symbol (ref, snapshot_id, stream_id, path, name, kind, language,
                                 line, col, end_line, end_col)
               VALUES ('symbol:w.py/value@snap:7', 7, 1, 'w.py', 'value', 'method', 'python', 20, 5, 20, 10),
                      ('symbol:w.py/value@snap:7', 7, 1, 'w.py', 'value', 'method', 'python', 10, 5, 10, 10),
                      ('symbol:w.py/spin@snap:7', 7, 1, 'w.py', 'spin', 'method', 'python', 30, 5, 30, 9);",
        )
        .unwrap();
        embedded::migrations::runner()
            .set_target(refinery::Target::Version(123))
            .run(&mut conn)
            .unwrap();
        let rows: Vec<(String, i64, i64)> = conn
            .prepare("SELECT ref, line, start_line FROM symbol ORDER BY line")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            rows,
            vec![
                ("symbol:w.py/value@snap:7".into(), 10, 10),
                ("symbol:w.py/value~2@snap:7".into(), 20, 20),
                ("symbol:w.py/spin@snap:7".into(), 30, 30),
            ]
        );
    }

    /// P3.2 (tsk472): V102 only adds — every agent-activity row survives,
    /// nudges written before it count as delivered, and the new anchors
    /// and uniqueness are in place.
    #[test]
    fn v102_adds_turn_anchors_and_keeps_every_row() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "foreign_keys", "ON").unwrap();
        embedded::migrations::runner()
            .set_target(refinery::Target::Version(101))
            .run(&mut conn)
            .unwrap();
        let now = "2026-09-29T00:00:00.000000Z";
        conn.execute_batch(&format!(
            "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
               VALUES (1, 'primary', 'a', 'main', 'r', 'r', '/r', '{now}', '{now}');
             INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
               VALUES (1, 1, 't', 'active', '{now}', '{now}');
             INSERT INTO effort (id, work_item, thread_id, started_at) VALUES (1, 'work_item:oxplow:tsk1', 1, '{now}');
             INSERT INTO agent_turn (id, thread_id, prompt, started_at) VALUES (1, 1, 'p', '{now}');
             INSERT INTO agent_nudge (thread_id, effort_id, kind, message, created_at)
               VALUES (1, 1, 'report-less-run', 'm', '{now}'), (1, 1, 'report-less-run', 'm', '{now}');
             INSERT INTO agent_token_usage (stream_id, thread_id, effort_id, session_id, agent_kind, provenance, recorded_at)
               VALUES (1, 1, 1, 's', 'claude', 'observed', '{now}');
             INSERT INTO agent_tool_call (thread_id, effort_id, tool, at) VALUES (1, 1, 'Edit', '{now}');
             INSERT INTO claim (thread_id, effort_id, statement, kind, created_at)
               VALUES (1, 1, 's', 'other', '{now}');
             INSERT INTO decision (thread_id, effort_id, question, choice, confidence, created_at)
               VALUES (1, 1, 'q', 'c', 'high', '{now}');
             INSERT INTO event_log (id, type, v, at, source, subject, payload)
               VALUES ('e0', 'config.changed', 1, '{now}', 'test', '[]', '{{}}');"
        ))
        .unwrap();

        migrate_and_compile(&mut conn).unwrap();

        let count = |sql: &str| -> i64 { conn.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(
            count("SELECT count(*) FROM agent_nudge WHERE delivered_at IS NULL"),
            0
        );
        assert_eq!(
            count("SELECT count(*) FROM v_agent_nudge"),
            2,
            "old duplicates kept"
        );
        for (view, column) in [
            ("v_tool_call", "turn_id"),
            ("v_token_usage", "turn_id"),
            ("v_agent_nudge", "turn_id"),
            ("v_decision", "turn_id"),
            ("v_claim", "turn_id"),
            ("v_event", "payload_expired_at"),
        ] {
            assert_eq!(
                count(&format!(
                    "SELECT count(*) FROM {view} WHERE {column} IS NULL"
                )),
                if view == "v_agent_nudge" {
                    2
                } else {
                    1 // v_event: the row logged before V102 isn't expired
                },
                "{view}.{column}"
            );
        }
        // A tool call projects once per event; a nudge fires once per cause.
        conn.execute(
            &format!("INSERT INTO agent_tool_call (thread_id, tool, at, turn_id, event_id) VALUES (1, 'Read', '{now}', 1, 'e1')"),
            [],
        )
        .unwrap();
        assert!(conn
            .execute(
                &format!("INSERT INTO agent_tool_call (thread_id, tool, at, event_id) VALUES (1, 'Read', '{now}', 'e1')"),
                [],
            )
            .is_err());
        conn.execute(
            &format!("INSERT INTO agent_nudge (thread_id, kind, message, created_at, cause) VALUES (1, 'k', 'm', '{now}', 'e1')"),
            [],
        )
        .unwrap();
        assert!(conn
            .execute(
                &format!("INSERT INTO agent_nudge (thread_id, kind, message, created_at, cause) VALUES (1, 'k', 'm', '{now}', 'e1')"),
                [],
            )
            .is_err());
        // Another kind for the same cause is its own nudge.
        conn.execute(
            &format!("INSERT INTO agent_nudge (thread_id, kind, message, created_at, cause, turn_id) VALUES (1, 'k2', 'm', '{now}', 'e1', 1)"),
            [],
        )
        .unwrap();
        // Deleting the turn keeps its rows, unanchored.
        conn.execute("DELETE FROM agent_turn WHERE id = 1", [])
            .unwrap();
        assert_eq!(
            count("SELECT count(*) FROM agent_tool_call WHERE event_id = 'e1' AND turn_id IS NULL"),
            1
        );
        assert_eq!(
            count("SELECT count(*) FROM agent_nudge WHERE kind = 'k2' AND turn_id IS NULL"),
            1
        );
        assert_eq!(count("SELECT count(*) FROM pragma_foreign_key_check"), 0);
        // Content is stored by hash; the view never exposes the bytes.
        conn.execute(
            &format!("INSERT INTO event_content (hash, namespace, bytes, size, created_at) VALUES ('h', 'agent', x'00', 1, '{now}')"),
            [],
        )
        .unwrap();
        assert_eq!(
            count("SELECT count(*) FROM pragma_table_info('v_event_content') WHERE name = 'bytes'"),
            0
        );
    }

    /// Regression: the first version of V18 rebuilt the `task` table
    /// via `task_new` + `DROP TABLE task` + rename, which under
    /// `PRAGMA foreign_keys = ON` cascaded and wiped every
    /// `task_effort` row (`task_effort.task_id REFERENCES task(id)
    /// ON DELETE CASCADE`). The fixed migration uses
    /// `ALTER TABLE … DROP COLUMN` instead, which leaves child rows
    /// untouched. This test asserts an `effort` row created
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
            "INSERT INTO effort (work_item, thread_id, started_at)
             VALUES ('work_item:oxplow:tsk' || ?1, 1, ?2)",
            (task_id, now),
        )
        .unwrap();

        // effort row survives alongside its parent.
        let n: i64 = conn
            .query_row(
                "SELECT count(*) FROM v_effort WHERE task_id = ?1",
                [task_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "effort row must coexist with its parent task");

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
