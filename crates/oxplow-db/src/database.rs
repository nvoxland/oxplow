use std::collections::HashMap;
use std::path::Path;
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
///
/// Every read here is a property of one measure's facts, so each measure has
/// its own entry and write generation: a fact write forgets only the
/// measures it wrote. A deletion (a prune) can't cheaply say which measures
/// lost facts, so it bumps the epoch, which every entry is read under.
#[derive(Default)]
pub struct QueryMemo {
    measures: Mutex<HashMap<i64, MeasureMemo>>,
    /// Bumped by a deletion of facts: everything memoized is forgotten.
    epoch: std::sync::atomic::AtomicU64,
}

/// What's memoized about one measure, and its write generation.
#[derive(Default)]
struct MeasureMemo {
    generation: u64,
    epoch: u64,
    /// The producers that have emitted facts for it (tsk130).
    producers: Option<Vec<String>>,
    /// Its distinct `(producer, rule, severity, dims_json)` slices.
    slice_keys: Option<Vec<crate::fact_store::FactSliceKey>>,
    /// One representative fact per slice.
    representatives: Option<Vec<crate::fact_store::FactRow>>,
}

/// The generation a memoized read was taken under: the epoch and the
/// measure's write generation.
pub(crate) type MemoGeneration = (u64, u64);

impl QueryMemo {
    /// A poisoned memo is not a reason to take the process down: it's a cache,
    /// and the worst a poisoned map holds is a value we'd have recomputed.
    fn measures(&self) -> std::sync::MutexGuard<'_, HashMap<i64, MeasureMemo>> {
        self.measures.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn epoch(&self) -> u64 {
        self.epoch.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Read `field` of `measure_id`'s entry, with the generation it's under —
    /// pass that back to [`Self::put`]. An entry from an earlier epoch reads
    /// as empty.
    fn get<T: Clone>(
        &self,
        measure_id: i64,
        field: impl Fn(&MeasureMemo) -> &Option<T>,
    ) -> (MemoGeneration, Option<T>) {
        let epoch = self.epoch();
        let memo = self.measures();
        let entry = memo.get(&measure_id);
        let generation = entry.map_or(0, |e| e.generation);
        let hit = entry
            .filter(|e| e.epoch == epoch)
            .and_then(|e| field(e).clone());
        ((epoch, generation), hit)
    }

    /// Memoize `value` into `field`, but **only if no fact write (to this
    /// measure) or deletion landed since `generation`**. Without that check a
    /// query that started before a write and finished after it would install
    /// a result missing the new rows, and nothing would clear it until the
    /// *next* write — a metric silently blind to a producer.
    fn put<T>(
        &self,
        measure_id: i64,
        generation: MemoGeneration,
        value: T,
        field: impl Fn(&mut MeasureMemo) -> &mut Option<T>,
    ) {
        if self.epoch() != generation.0 {
            return;
        }
        let mut memo = self.measures();
        let entry = memo.entry(measure_id).or_default();
        if entry.generation != generation.1 {
            return;
        }
        if entry.epoch != generation.0 {
            // A stale epoch's fields are forgotten as the entry is reused.
            *entry = MeasureMemo {
                generation: entry.generation,
                epoch: generation.0,
                ..MeasureMemo::default()
            };
        }
        *field(entry) = Some(value);
    }

    pub(crate) fn producers_get(&self, measure_id: i64) -> (MemoGeneration, Option<Vec<String>>) {
        self.get(measure_id, |e| &e.producers)
    }

    pub(crate) fn producers_put(
        &self,
        measure_id: i64,
        generation: MemoGeneration,
        value: Vec<String>,
    ) {
        self.put(measure_id, generation, value, |e| &mut e.producers)
    }

    pub(crate) fn slice_keys_get(
        &self,
        measure_id: i64,
    ) -> (MemoGeneration, Option<Vec<crate::fact_store::FactSliceKey>>) {
        self.get(measure_id, |e| &e.slice_keys)
    }

    pub(crate) fn slice_keys_put(
        &self,
        measure_id: i64,
        generation: MemoGeneration,
        value: Vec<crate::fact_store::FactSliceKey>,
    ) {
        self.put(measure_id, generation, value, |e| &mut e.slice_keys)
    }

    pub(crate) fn representatives_get(
        &self,
        measure_id: i64,
    ) -> (MemoGeneration, Option<Vec<crate::fact_store::FactRow>>) {
        self.get(measure_id, |e| &e.representatives)
    }

    pub(crate) fn representatives_put(
        &self,
        measure_id: i64,
        generation: MemoGeneration,
        value: Vec<crate::fact_store::FactRow>,
    ) {
        self.put(measure_id, generation, value, |e| &mut e.representatives)
    }

    /// Called after facts for `measures` are committed: bump those measures'
    /// generations (so an in-flight read of one declines to cache) and drop
    /// what's memoized for them. Other measures keep theirs.
    pub(crate) fn invalidate_measures(&self, measures: impl IntoIterator<Item = i64>) {
        let mut memo = self.measures();
        for m in measures {
            let entry = memo.entry(m).or_default();
            *entry = MeasureMemo {
                generation: entry.generation + 1,
                epoch: entry.epoch,
                ..MeasureMemo::default()
            };
        }
    }

    /// Called after facts were deleted: forget everything.
    pub(crate) fn invalidate_all(&self) {
        self.epoch.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
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

/// Refuse a SQLite built without its math functions (`log2`, `ln`, `pow`,
/// …): SQL models and lenses may use them (P7.B5: oxplow-bundled's
/// `change_interest` scores with `log2`), and without them such a model
/// only surfaces as an extension's compile error. The bundled SQLite gets
/// them from `LIBSQLITE3_FLAGS = -DSQLITE_ENABLE_MATH_FUNCTIONS` in
/// `.cargo/config.toml`, which a build from outside this tree doesn't
/// read — so every open checks, and says how to build (tsk726).
fn require_math_functions(c: &Connection) -> Result<(), DbInitError> {
    c.query_row("SELECT log2(8)", [], |r| r.get::<_, f64>(0))
        .map(|_| ())
        .map_err(|e| {
            DbInitError::Build(format!(
                "this oxplow's SQLite has no math functions ({e}); build it with \
                 LIBSQLITE3_FLAGS=-DSQLITE_ENABLE_MATH_FUNCTIONS (the repo's .cargo/config.toml \
                 sets it)"
            ))
        })
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
        require_math_functions(&probe)?;
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
        require_math_functions(&setup)?;
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
        require_math_functions(&conn).expect("SQLite has its math functions");
        load_migrated(&mut conn).expect("in-memory migrations and models");
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
        self.transaction(move |tx| crate::models::compile_extensions(tx, &extensions))
            .await
    }

    /// Check the extensions' models without publishing them — works on a
    /// read-only database ([`crate::models::check_extensions`]); `stubs`
    /// stand in for declared entities that haven't synced.
    pub async fn check_extension_models(
        &self,
        extensions: Vec<crate::models::ExtensionModels>,
        stubs: Vec<crate::models::EntityStub>,
    ) -> Result<crate::models::CheckedModels, oxplow_domain::DomainError> {
        self.read(move |tx| crate::models::check_extensions(tx, &extensions, &stubs))
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
                // So Busy comes at BEGIN, and is retried like any other
                // (tsk1005).
                let outcome = conn
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                    .map_err(map_sql_err)
                    .and_then(|tx| {
                        let value = f(&tx)?;
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
    /// This build of oxplow lacks something the database needs.
    #[error("build: {0}")]
    Build(String),
}

/// Bring a database to this build: drop the model views, apply the
/// migrations, then compile the models (P4.2). The views are recreated at
/// every open, so a migration never works around one — and never creates
/// one: a published view is a model file.
/// `conn` (empty) as `migrate_and_compile` would leave it, restored from a
/// migrated template kept on disk: 128 migrations and the core models
/// cost ~300 ms, and every test builds a fresh database — in its own
/// process under nextest, so only a file outlives one. The template is
/// keyed by this executable's build and today's date (a model twin's
/// keep-until compares against today), written once and atomically.
/// Without a usable temp dir it migrates directly.
fn load_migrated(conn: &mut Connection) -> Result<(), DbInitError> {
    let Some(path) = template_path() else {
        return migrate_and_compile(conn);
    };
    if !path.exists() {
        write_template(&path)?;
    }
    match conn.restore(
        rusqlite::MAIN_DB,
        &path,
        None::<fn(rusqlite::backup::Progress)>,
    ) {
        Ok(()) => Ok(()),
        // An unreadable template is dropped; this one migrates directly.
        Err(_) => {
            let _ = std::fs::remove_file(&path);
            migrate_and_compile(conn)
        }
    }
}

/// Where this build's migrated template lives:
/// `<temp>/oxplow-db-templates/<build + date>.sqlite`.
fn template_path() -> Option<std::path::PathBuf> {
    use std::hash::{Hash, Hasher};
    let exe = std::env::current_exe().ok()?;
    let meta = std::fs::metadata(&exe).ok()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    exe.hash(&mut h);
    meta.len().hash(&mut h);
    meta.modified().ok()?.hash(&mut h);
    crate::models::today().hash(&mut h);
    Some(
        std::env::temp_dir()
            .join("oxplow-db-templates")
            .join(format!("{:016x}.sqlite", h.finish())),
    )
}

/// Write a new template at `path` (through a temporary file renamed into
/// place, so a concurrent reader never sees half of one), and drop
/// templates older than a day.
fn write_template(path: &std::path::Path) -> Result<(), DbInitError> {
    let fail = |e: String| DbInitError::Migration(format!("migrated template: {e}"));
    let dir = path.parent().ok_or_else(|| fail("no parent".into()))?;
    std::fs::create_dir_all(dir).map_err(|e| fail(e.to_string()))?;
    let tmp = dir.join(format!(".{}.tmp", uuid::Uuid::new_v4()));
    // Migrated in memory, as a test database is (a migration's journal
    // mode switch is refused on a file inside its transaction), then
    // copied out page for page.
    let mut conn = Connection::open_in_memory().map_err(|e| fail(e.to_string()))?;
    migrate_and_compile(&mut conn)?;
    conn.backup(
        rusqlite::MAIN_DB,
        &tmp,
        None::<fn(rusqlite::backup::Progress)>,
    )
    .map_err(|e| fail(e.to_string()))?;
    std::fs::rename(&tmp, path).map_err(|e| fail(e.to_string()))?;
    let day = std::time::Duration::from_secs(24 * 3600);
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > day);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    Ok(())
}

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

    /// A database restored from the migrated template is exactly what
    /// migrating from scratch makes: same tables, views, indexes and
    /// triggers, same migration history.
    #[test]
    fn an_in_memory_database_matches_a_fresh_migration() {
        let schema = |c: &Connection| -> Vec<(String, String, Option<String>)> {
            let mut stmt = c
                .prepare("SELECT type, name, sql FROM sqlite_master ORDER BY type, name")
                .unwrap();
            stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        let history = |c: &Connection| -> i64 {
            c.query_row("SELECT count(*) FROM refinery_schema_history", [], |r| {
                r.get(0)
            })
            .unwrap()
        };
        let mut fresh = Connection::open_in_memory().unwrap();
        migrate_and_compile(&mut fresh).unwrap();
        for _ in 0..2 {
            let db = Database::in_memory();
            let conn = db.conn().unwrap();
            assert_eq!(schema(&conn), schema(&fresh));
            assert_eq!(history(&conn), history(&fresh));
        }
        assert!(
            template_path().is_some_and(|p| p.exists()),
            "the template is kept"
        );
    }

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

    /// tsk978: `Database::transaction` is the one write path — it begins
    /// IMMEDIATE and retries Busy. No store opens a transaction of its own
    /// on a raw connection; the open-time model compile (`models.rs`, run
    /// before the pool serves anyone) and this file's own are the two.
    #[test]
    fn every_store_write_runs_in_the_retried_transaction() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![src.clone()];
        let mut own = Vec::new();
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|e| e != "rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                let production = text.split("#[cfg(test)]").next().unwrap_or_default();
                let rel = path
                    .strip_prefix(&src)
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                if production.contains(".transaction()")
                    && !matches!(rel.as_str(), "database.rs" | "models.rs")
                {
                    own.push(rel);
                }
            }
        }
        own.sort();
        assert_eq!(own, Vec::<String>::new());
    }

    /// tsk1005: under IMMEDIATE, Busy comes at `BEGIN` — another writer
    /// holds the lock past `busy_timeout` — and that is retried like a
    /// Busy inside the transaction.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_transaction_that_cant_begin_is_retried() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("busy.sqlite");
        let db = Database::open(&path).unwrap();
        db.transaction(|tx| {
            tx.execute("CREATE TABLE t (x INTEGER)", [])
                .map_err(map_sql_err)?;
            Ok(())
        })
        .await
        .unwrap();
        // Another writer holds the lock past the pool's 5 s wait.
        let (held, holding) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("BEGIN IMMEDIATE").unwrap();
            held.send(()).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(6_500));
            conn.execute_batch("COMMIT").unwrap();
        });
        holding.recv().unwrap();
        db.transaction(|tx| {
            tx.execute("INSERT INTO t VALUES (1)", [])
                .map_err(map_sql_err)?;
            Ok(())
        })
        .await
        .expect("retried once the lock was free");
        holder.join().unwrap();
    }

    /// A rehearsal keeps nothing it wrote, and absorbs a busy blip like
    /// a real transaction.
    #[tokio::test]
    async fn rehearse_rolls_back_and_retries_busy() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        let db = Database::in_memory();
        let calls = Arc::new(AtomicU32::new(0));
        let seen = calls.clone();
        let out = db
            .rehearse(move |tx| {
                if seen.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Err(oxplow_domain::DomainError::Busy("locked".into()));
                }
                tx.execute("CREATE TABLE rehearsed (x INTEGER)", [])
                    .map_err(map_sql_err)?;
                tx.execute("INSERT INTO rehearsed VALUES (1)", [])
                    .map_err(map_sql_err)?;
                Ok(7)
            })
            .await
            .unwrap();
        assert_eq!(out, 7);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        let kept: i64 = db
            .conn()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'rehearsed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(kept, 0, "rolled back");
    }

    #[test]
    fn every_connection_has_sqlite_math_functions() {
        let conn = Connection::open_in_memory().unwrap();
        require_math_functions(&conn).unwrap();
        let (log2, pow): (f64, f64) = conn
            .query_row("SELECT log2(8), pow(2, 10)", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!((log2, pow), (3.0, 1024.0));
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
        // Not a database, or another schema version: refused, file untouched.
        std::fs::write(&path, "junk").unwrap();
        assert!(Database::open_read_only(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"junk");
        std::fs::remove_file(&path).unwrap();
        drop(Database::open(&path).unwrap());
        {
            // Recorded by some other build.
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute(
                "UPDATE refinery_schema_history SET version = 99
                 WHERE version = (SELECT max(version) FROM refinery_schema_history)",
                [],
            )
            .unwrap();
        }
        let err = match Database::open_read_only(&path) {
            Ok(_) => panic!("another schema version must be refused"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("schema version 99"), "{err}");
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

    /// V5: an effort's work item becomes optional and at most one effort
    /// is open per thread. A V4 database with two open efforts on one
    /// thread (and an unlinked one stored as `''`) keeps every effort and
    /// its files; only the newest open one per thread stays open.
    #[test]
    fn v5_keeps_efforts_and_leaves_one_open_per_thread() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::models::drop_all(&conn).unwrap();
        embedded::migrations::runner()
            .set_target(refinery::Target::Version(4))
            .run(&mut conn)
            .unwrap();
        let now = "2026-04-29T00:00:00Z";
        conn.execute_batch(&format!(
            "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
               VALUES (1, 'primary', 'a', 'main', 'r', 'r', '/r', '{now}', '{now}');
             INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
               VALUES (1, 1, 't', 'active', '{now}', '{now}');
             INSERT INTO effort (id, work_item, thread_id, started_at)
               VALUES (1, 'work_item:oxplow:tsk1', 1, '2026-04-29T00:00:01Z'),
                      (2, '', 1, '2026-04-29T00:00:02Z');
             INSERT INTO effort_file (effort_id, path, change_kind)
               VALUES (1, 'src/a.rs', 'updated');"
        ))
        .unwrap();
        embedded::migrations::runner().run(&mut conn).unwrap();
        let efforts: Vec<(i64, Option<String>, bool, Option<String>)> = conn
            .prepare("SELECT id, work_item, ended_at IS NULL, closed_by FROM effort ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            efforts,
            vec![
                (
                    1,
                    Some("work_item:oxplow:tsk1".to_string()),
                    false,
                    Some("system".to_string())
                ),
                (2, None, true, None),
            ]
        );
        let files: i64 = conn
            .query_row(
                "SELECT count(*) FROM effort_file WHERE effort_id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(files, 1);
        // A second open effort on the thread is refused.
        let err = conn.execute(
            "INSERT INTO effort (thread_id, started_at) VALUES (1, '2026-04-29T00:00:03Z')",
            [],
        );
        assert!(err.is_err(), "one open effort per thread");
    }

    /// V6: `work_item.created` and `work_item.transitioned` lose the
    /// `effort` a task's status used to open; the logged payloads drop it
    /// too, so they read under the narrowed v1 type.
    #[test]
    fn v6_strips_effort_from_logged_work_item_payloads() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::models::drop_all(&conn).unwrap();
        embedded::migrations::runner()
            .set_target(refinery::Target::Version(5))
            .run(&mut conn)
            .unwrap();
        conn.execute_batch(
            r#"INSERT INTO event_log (id, type, v, at, source, subject, payload) VALUES
                 ('a', 'work_item.created', 1, '2026-04-29T00:00:00Z', 'human', '[]',
                  '{"work_item":"work_item:oxplow:tsk1","status":"in_progress","effort":"effort:eff1"}'),
                 ('b', 'work_item.transitioned', 1, '2026-04-29T00:00:01Z', 'human', '[]',
                  '{"work_item":"work_item:oxplow:tsk1","from":"in_progress","to":"done","effort":"effort:eff1"}'),
                 ('c', 'effort.linked', 1, '2026-04-29T00:00:02Z', 'human', '[]',
                  '{"effort":"effort:eff1","work_item":"work_item:oxplow:tsk1"}');"#,
        )
        .unwrap();
        embedded::migrations::runner().run(&mut conn).unwrap();
        let payloads: Vec<String> = conn
            .prepare("SELECT payload FROM event_log ORDER BY seq")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            payloads,
            vec![
                r#"{"work_item":"work_item:oxplow:tsk1","status":"in_progress"}"#,
                r#"{"work_item":"work_item:oxplow:tsk1","from":"in_progress","to":"done"}"#,
                r#"{"effort":"effort:eff1","work_item":"work_item:oxplow:tsk1"}"#,
            ]
        );
    }

    /// V8: effort events are v2 only and lose `retroactive`.
    #[test]
    fn v8_moves_effort_events_to_v2_without_retroactive() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::models::drop_all(&conn).unwrap();
        embedded::migrations::runner()
            .set_target(refinery::Target::Version(7))
            .run(&mut conn)
            .unwrap();
        conn.execute_batch(
            r#"INSERT INTO event_log (id, type, v, at, source, subject, payload) VALUES
                 ('a', 'effort.opened', 1, '2026-04-29T00:00:00Z', 'human', '[]',
                  '{"effort":"effort:eff1","work_item":"work_item:oxplow:tsk1","thread":"thread:thr1"}'),
                 ('b', 'effort.closed', 2, '2026-04-29T00:00:01Z', 'human', '[]',
                  '{"effort":"effort:eff1","retroactive":true}'),
                 ('c', 'effort.finished', 2, '2026-04-29T00:00:02Z', 'human', '[]',
                  '{"effort":"effort:eff1"}');"#,
        )
        .unwrap();
        embedded::migrations::runner().run(&mut conn).unwrap();
        let rows: Vec<(i64, String)> = conn
            .prepare("SELECT v, payload FROM event_log ORDER BY seq")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            rows,
            vec![
                (
                    2,
                    r#"{"effort":"effort:eff1","work_item":"work_item:oxplow:tsk1","thread":"thread:thr1"}"#
                        .to_string()
                ),
                (2, r#"{"effort":"effort:eff1"}"#.to_string()),
                (2, r#"{"effort":"effort:eff1"}"#.to_string()),
            ]
        );
    }

    /// V9: once-marks are kept per thread, and per effort within it; the
    /// effort marks already fired move over under their effort's thread.
    #[test]
    fn v9_keeps_effort_marks_under_their_thread() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::models::drop_all(&conn).unwrap();
        embedded::migrations::runner()
            .set_target(refinery::Target::Version(8))
            .run(&mut conn)
            .unwrap();
        conn.execute_batch(
            "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
               VALUES (1, 'primary', 'p', 'main', 'r', 'r', '/r', '2026-01-01', '2026-01-01');
             INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
               VALUES (7, 1, 't', 'active', '2026-01-01', '2026-01-01');
             INSERT INTO effort (id, thread_id, started_at) VALUES (3, 7, '2026-01-01');
             INSERT INTO effort_once_mark (effort_id, mark, fired_at)
               VALUES (3, 'report-less-run', '2026-01-02');",
        )
        .unwrap();
        embedded::migrations::runner().run(&mut conn).unwrap();
        let rows: Vec<(i64, Option<i64>, String)> = conn
            .prepare("SELECT thread_id, effort_id, mark FROM once_mark")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows, vec![(7, Some(3), "report-less-run".to_string())]);
        conn.execute(
            "INSERT INTO once_mark (thread_id, effort_id, mark, fired_at) VALUES (7, NULL, 'x', 'n')",
            [],
        )
        .unwrap();
        assert!(
            conn.execute(
                "INSERT INTO once_mark (thread_id, effort_id, mark, fired_at) VALUES (7, NULL, 'x', 'n')",
                [],
            )
            .is_err(),
            "a thread mark fires once"
        );
    }

    /// V10: a nudge has an audience, the agent's for every nudge so far;
    /// each hint's evaluations are counted per thread.
    #[test]
    fn v10_gives_nudges_an_audience_and_counts_evaluations() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::models::drop_all(&conn).unwrap();
        embedded::migrations::runner()
            .set_target(refinery::Target::Version(9))
            .run(&mut conn)
            .unwrap();
        conn.execute_batch(
            "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
               VALUES (1, 'primary', 'p', 'main', 'r', 'r', '/r', '2026-01-01', '2026-01-01');
             INSERT INTO threads (id, stream_id, title, status, created_at, updated_at)
               VALUES (7, 1, 't', 'active', '2026-01-01', '2026-01-01');
             INSERT INTO agent_nudge (thread_id, kind, message, created_at)
               VALUES (7, 'report-less-run', 'm', '2026-01-02');",
        )
        .unwrap();
        embedded::migrations::runner().run(&mut conn).unwrap();
        let audience: String = conn
            .query_row("SELECT audience FROM agent_nudge", [], |r| r.get(0))
            .unwrap();
        assert_eq!(audience, "agent");
        conn.execute(
            "INSERT INTO hint_stat (thread_id, hint, evaluated, last_evaluated_at) VALUES (7, 'x/a', 1, 'n')",
            [],
        )
        .unwrap();
    }

    /// V13: changes analyzed before it count as scanned, so the first boot
    /// doesn't rescan every one.
    #[test]
    fn v13_counts_earlier_analyses_as_scanned() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::models::drop_all(&conn).unwrap();
        embedded::migrations::runner()
            .set_target(refinery::Target::Version(12))
            .run(&mut conn)
            .unwrap();
        conn.execute_batch(
            "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
               VALUES (1, 'primary', 'p', 'main', 'r', 'r', '/r', '2026-01-01', '2026-01-01');
             INSERT INTO change (id, stream_id, kind, target, head_revision, status, events_to)
               VALUES (1, 1, 'working', '1', 'working', 'done', 40),
                      (2, 1, 'working', '2', 'working', 'pending', NULL);",
        )
        .unwrap();
        embedded::migrations::runner().run(&mut conn).unwrap();
        let rows: Vec<(i64, Option<i64>)> = conn
            .prepare("SELECT id, duplicates_events_to FROM change ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(rows, vec![(1, Some(40)), (2, None)]);
    }

    /// V14: config.changed is v2, naming the layer; every logged change
    /// was the project's.
    #[test]
    fn v14_moves_config_changes_to_v2_in_the_project_layer() {
        let mut conn = Connection::open_in_memory().unwrap();
        crate::models::drop_all(&conn).unwrap();
        embedded::migrations::runner()
            .set_target(refinery::Target::Version(13))
            .run(&mut conn)
            .unwrap();
        conn.execute_batch(
            r#"INSERT INTO event_log (id, type, v, at, source, subject, payload) VALUES
                 ('a', 'config.changed', 1, '2026-04-29T00:00:00Z', 'human', '[]',
                  '{"key":"zones","before":null,"after":[]}');"#,
        )
        .unwrap();
        embedded::migrations::runner().run(&mut conn).unwrap();
        let (v, payload): (i64, String) = conn
            .query_row("SELECT v, payload FROM event_log", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .unwrap();
        assert_eq!(v, 2);
        assert_eq!(
            payload,
            r#"{"key":"zones","before":null,"after":[],"layer":"project"}"#
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
