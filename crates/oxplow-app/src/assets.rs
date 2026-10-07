//! Assets (P7.B1, `.context/semantic-layer.md` "Assets"): derived data
//! whose inputs are tables, recomputed when a commit touches one.
//!
//! Two spine mechanisms, chosen by what a computation reads
//! (`.context/semantic-layer.md` "Assets"): inputs that are **tables** make an
//! asset — level-triggered by the database's own change notifications
//! (`Database::subscribe_changes`), recomputed in its own transactions,
//! with a persisted `asset_state` row; inputs from **outside the tables**
//! (a snapshot's blobs, a program, a provider) are ingestion — a
//! collector or a pump consumer.
//!
//! A [`Materializer`] names its asset and input tables and recomputes it.
//! The [`Assets`] runner hears which tables each commit touched (from the
//! one change loop in `models_changed.rs`), marks the materializers that
//! read them dirty, and recomputes each once its inputs have been quiet
//! for the coalesce window (a sweep's burst of writes is one recompute,
//! not one per commit) — then records `asset_state { computed_at,
//! events_to, snapshot_id, elapsed_ms }`. A recompute that fails is
//! logged; the next change tries again.
//!
//! An asset on a **clock** (`every()`, a model's `materialize: { every: 1h }`,
//! P8.B2) ignores its inputs' changes: it recomputes when its last
//! recompute (`asset_state.computed_at`, persisted) is `every` old — at
//! once when it never ran or is overdue, so a restart doesn't rebuild a
//! fresh one — and every `every` after.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use oxplow_db::Database;
use oxplow_domain::{DomainError, Timestamp};
use tokio::sync::Notify;

/// How long an asset's inputs must be quiet before it recomputes.
pub const COALESCE: Duration = Duration::from_secs(1);

/// What a recompute covered, beyond when it ran.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recomputed {
    /// The snapshot it was computed against, when it has one.
    pub snapshot_id: Option<i64>,
    /// For a materialized model: `full` or `incremental`.
    pub mode: Option<&'static str>,
    /// For an incremental model: the highest watermark value it holds.
    pub watermark: Option<i64>,
    /// For a materialized model: how many rows it holds.
    pub row_count: Option<i64>,
}

/// One asset: its name, the tables it reads, and how to recompute it.
#[async_trait]
pub trait Materializer: Send + Sync {
    /// `metric_cube`, or a materialized model's view.
    fn asset(&self) -> &str;
    /// The tables whose commits make it stale — or another asset by name:
    /// this one then recomputes after that one does, never while that one
    /// is pending (its first build waits for the other's).
    fn inputs(&self) -> Vec<String>;
    /// Recomputed on this clock instead of on its inputs' changes.
    fn every(&self) -> Option<Duration> {
        None
    }
    /// A fingerprint of what it computes (a model's SELECT), recorded with
    /// each recompute: a clocked asset whose definition changed is due at
    /// once. `None` when there's nothing to tell apart.
    fn definition(&self) -> Option<String> {
        None
    }
    /// `full`: refill whole — an input saw a rewrite, or this is the first
    /// build since it registered. Only an incremental model does less
    /// otherwise.
    async fn recompute(&self, full: bool) -> Result<Recomputed, DomainError>;
}

struct Entry {
    materializer: Arc<dyn Materializer>,
    inputs: BTreeSet<String>,
    dirty: Arc<Notify>,
    /// Sticky until a recompute succeeds: an input was rewritten (an
    /// UPDATE or a DELETE, not only inserts) or it hasn't built since it
    /// registered (an open, a contract change).
    needs_full: Arc<std::sync::atomic::AtomicBool>,
    /// Its recompute loop, stopped when the asset goes.
    task: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Recomputes asked for, and asked-for ones done: pending while
    /// `done < wanted`. An asset reading this one waits until it isn't.
    wanted: std::sync::atomic::AtomicU64,
    done: std::sync::atomic::AtomicU64,
    /// Woken after each recompute.
    settled: Notify,
}

impl Entry {
    fn mark(&self) {
        self.wanted
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.dirty.notify_one();
    }

    fn pending(&self) -> bool {
        self.done.load(std::sync::atomic::Ordering::SeqCst)
            < self.wanted.load(std::sync::atomic::Ordering::SeqCst)
    }
}

type Entries = Arc<std::sync::RwLock<Vec<Arc<Entry>>>>;

/// Wait until no asset `entry` reads is pending.
async fn wait_for_upstream(entries: &Entries, entry: &Entry) {
    loop {
        let upstream: Vec<Arc<Entry>> = entries
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter(|e| {
                e.materializer.asset() != entry.materializer.asset()
                    && entry.inputs.contains(e.materializer.asset())
            })
            .cloned()
            .collect();
        let Some(up) = upstream.iter().find(|e| e.pending()) else {
            return;
        };
        let settled = up.settled.notified();
        if up.pending() {
            // Bounded, so a wake-up between the check and the wait is
            // never lost for long.
            let _ = tokio::time::timeout(Duration::from_millis(250), settled).await;
        }
    }
}

/// Recompute `entry` once nothing it reads is pending, then tell the
/// assets that read it.
async fn run_once(db: &Database, entries: &Entries, entry: &Entry, now: &Now) {
    wait_for_upstream(entries, entry).await;
    let asked = entry.wanted.load(std::sync::atomic::Ordering::SeqCst);
    recompute(db, entry, now).await;
    entry
        .done
        .fetch_max(asked, std::sync::atomic::Ordering::SeqCst);
    entry.settled.notify_waiters();
    let asset = entry.materializer.asset();
    for downstream in entries.read().unwrap_or_else(|e| e.into_inner()).iter() {
        if downstream.materializer.asset() != asset && downstream.inputs.contains(asset) {
            downstream.mark();
        }
    }
}

/// A materialized model as the registry has it: its compiled SELECT, its
/// contract (the columns its table is made of) and the tables it reads,
/// transitively. Any of them changing re-registers it, so a table the
/// compiler recreated — its contract changed though its SELECT didn't (an
/// input's column changed type) — gets its first build.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ModelAsset {
    sql: String,
    contract: Option<String>,
    tables: Vec<String>,
    /// Its policy as recorded (`on_change`, `every 1h`).
    materialize: String,
}

/// What time it is, for an asset's clock and its records.
pub type Now = Arc<dyn Fn() -> Timestamp + Send + Sync>;

/// The registered assets. Cloning shares them.
#[derive(Clone)]
pub struct Assets {
    db: Database,
    coalesce: Duration,
    now: Now,
    entries: Entries,
    /// The materialized models registered from the model registry
    /// ([`Assets::sync_models`]), by view.
    models: Arc<tokio::sync::Mutex<BTreeMap<String, ModelAsset>>>,
    /// The searchable plugin ref kinds whose index is registered
    /// ([`Assets::sync_search_kinds`], `kind_search`), by kind.
    pub(crate) search_kinds:
        Arc<tokio::sync::Mutex<BTreeMap<String, crate::kind_search::SearchableKind>>>,
}

impl Assets {
    pub fn new(db: Database, coalesce: Duration) -> Self {
        Self {
            db,
            coalesce,
            now: Arc::new(Timestamp::now),
            entries: Arc::default(),
            models: Arc::default(),
            search_kinds: Arc::default(),
        }
    }

    /// The database its assets are computed in.
    pub(crate) fn db(&self) -> &Database {
        &self.db
    }

    /// These assets on clock `now` (tests: one that follows tokio's
    /// paused clock).
    pub fn with_now(self, now: Now) -> Self {
        Self { now, ..self }
    }

    /// Add `materializer` and start its recompute loop: once now (its
    /// first build), then after each quiet burst of changes to its inputs.
    pub fn register(&self, materializer: Arc<dyn Materializer>) {
        let entry = Arc::new(Entry {
            inputs: materializer.inputs().into_iter().collect(),
            materializer,
            dirty: Arc::new(Notify::new()),
            needs_full: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            task: std::sync::Mutex::new(None),
            // Its first build is asked for.
            wanted: std::sync::atomic::AtomicU64::new(1),
            done: std::sync::atomic::AtomicU64::new(0),
            settled: Notify::new(),
        });
        self.entries
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .push(entry.clone());
        let (db, coalesce, now, looping, entries) = (
            self.db.clone(),
            self.coalesce,
            self.now.clone(),
            entry.clone(),
            self.entries.clone(),
        );
        let task = tokio::spawn(async move {
            let entry = looping;
            if let Some(every) = entry.materializer.every() {
                let mut wait = until_due(
                    &db,
                    entry.materializer.asset(),
                    entry.materializer.definition(),
                    every,
                    now(),
                )
                .await;
                loop {
                    tokio::time::sleep(wait).await;
                    entry.mark();
                    run_once(&db, &entries, &entry, &now).await;
                    wait = every;
                }
            }
            run_once(&db, &entries, &entry, &now).await;
            loop {
                entry.dirty.notified().await;
                // Wait until its inputs have been quiet for the window.
                while tokio::time::timeout(coalesce, entry.dirty.notified())
                    .await
                    .is_ok()
                {}
                // A wake-up the last recompute already covered (it asked
                // for this one before running) needs nothing.
                if entry.pending() {
                    run_once(&db, &entries, &entry, &now).await;
                }
            }
        });
        *entry.task.lock().unwrap_or_else(|e| e.into_inner()) = Some(task);
    }

    /// Stop and forget `asset` (a materialized model no longer published).
    pub fn remove(&self, asset: &str) {
        let mut entries = self.entries.write().unwrap_or_else(|e| e.into_inner());
        entries.retain(|e| {
            let keep = e.materializer.asset() != asset;
            if !keep {
                if let Some(task) = e.task.lock().unwrap_or_else(|e| e.into_inner()).take() {
                    task.abort();
                }
            }
            keep
        });
    }

    /// Keep one [`SqlModelMaterializer`] per model the registry says is
    /// `materialize: on_change` (P7.B2): new ones registered (their first
    /// recompute fills the table), gone or changed ones replaced or
    /// stopped. Run when the registry changes.
    pub async fn sync_models(&self) -> Result<(), DomainError> {
        let wanted = materialized_models(&self.db).await?;
        let mut have = self.models.lock().await;
        for (view, old) in have.clone() {
            if wanted.get(&view) != Some(&old) {
                self.remove(&view);
                have.remove(&view);
            }
        }
        for (view, model) in wanted {
            if have.contains_key(&view) {
                continue;
            }
            let policy = oxplow_db::models::recorded_materialize(&model.materialize)
                .unwrap_or(oxplow_db::models::Materialize::ON_CHANGE);
            self.register(Arc::new(SqlModelMaterializer::new(
                self.db.clone(),
                &view,
                &model.sql,
                model.tables.clone(),
                &policy,
            )));
            have.insert(view, model);
        }
        Ok(())
    }

    /// A commit changed some tables: mark every asset reading one dirty.
    pub fn changed(&self, changed: &oxplow_db::changes::Changed) {
        for entry in self
            .entries
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            if entry.inputs.iter().any(|t| changed.rewrote.contains(t)) {
                entry
                    .needs_full
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
            if entry.inputs.iter().any(|t| changed.contains(t)) {
                entry.mark();
            }
        }
    }

    /// Anything may have changed (a lagged listener): every asset is dirty.
    pub fn all_changed(&self) {
        for entry in self
            .entries
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            entry
                .needs_full
                .store(true, std::sync::atomic::Ordering::SeqCst);
            entry.mark();
        }
    }
}

/// The model registry as an asset reads it: each model's compiled SELECT,
/// its policy when materialized, its contract, and what each reads.
struct Registry {
    /// view → (sql, materialize, contract columns).
    models: BTreeMap<String, (String, Option<String>, Option<String>)>,
    /// view → the views and tables it reads.
    inputs: BTreeMap<String, Vec<String>>,
}

impl Registry {
    fn load(tx: &rusqlite::Connection) -> Result<Registry, DomainError> {
        let mut st = tx
            .prepare(
                "SELECT m.view, m.sql, m.materialize, c.columns_json FROM model m
                 LEFT JOIN model_contract c ON c.view = m.view AND c.version = m.version",
            )
            .map_err(oxplow_db::map_sql_err)?;
        let models = st
            .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?, r.get(3)?))))
            .map_err(oxplow_db::map_sql_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(oxplow_db::map_sql_err)?;
        let mut st = tx
            .prepare("SELECT view, input FROM model_input")
            .map_err(oxplow_db::map_sql_err)?;
        let mut inputs: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for row in st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(oxplow_db::map_sql_err)?
        {
            let (view, input) = row.map_err(oxplow_db::map_sql_err)?;
            inputs.entry(view).or_default().push(input);
        }
        Ok(Registry { models, inputs })
    }

    /// The tables `view`'s SELECT reads: its inputs followed through live
    /// models to tables — and to a materialized model's own table, which
    /// is where that one's changes come from.
    fn input_tables(&self, view: &str) -> Vec<String> {
        let mut tables = BTreeSet::new();
        let mut seen = BTreeSet::new();
        let mut todo: Vec<&String> = self.inputs.get(view).into_iter().flatten().collect();
        while let Some(input) = todo.pop() {
            if !seen.insert(input.clone()) {
                continue;
            }
            match self.models.get(input) {
                Some((_, Some(_), _)) => {
                    tables.insert(oxplow_db::models::materialized_table(input));
                }
                Some((_, None, _)) => todo.extend(self.inputs.get(input).into_iter().flatten()),
                None => {
                    tables.insert(input.clone());
                }
            }
        }
        tables.into_iter().collect()
    }
}

/// The registry's materialized models, with the tables each reads.
async fn materialized_models(db: &Database) -> Result<BTreeMap<String, ModelAsset>, DomainError> {
    db.read(|tx| {
        let registry = Registry::load(tx)?;
        Ok(registry
            .models
            .iter()
            .filter_map(|(view, (sql, materialize, contract))| {
                Some((
                    view.clone(),
                    ModelAsset {
                        sql: sql.clone(),
                        contract: contract.clone(),
                        tables: registry.input_tables(view),
                        materialize: materialize.clone()?,
                    },
                ))
            })
            .collect())
    })
    .await
}

/// The tables whose commits change what each of `views` returns — a
/// materialized one's own table, a live one's inputs — for those the
/// registry lists (an unpublished view has no entry).
pub(crate) async fn tables_behind(
    db: &Database,
    views: &[String],
) -> Result<BTreeMap<String, Vec<String>>, DomainError> {
    let views = views.to_vec();
    db.read(move |tx| {
        let registry = Registry::load(tx)?;
        Ok(views
            .into_iter()
            .filter_map(|view| {
                let tables = match registry.models.get(&view)? {
                    (_, Some(_), _) => vec![oxplow_db::models::materialized_table(&view)],
                    (_, None, _) => registry.input_tables(&view),
                };
                Some((view, tables))
            })
            .collect())
    })
    .await
}

/// A materialized model (P7.B2): its table refilled, whole, from its
/// SELECT in one transaction — when an input changes, or on its `every:`
/// clock (P8.B2). An incremental one (P8.B4) appends the rows past its
/// watermark — its key — instead, unless a refill is asked for. Either
/// fails on SQL that emits one key twice; the failure is the asset's in
/// `asset_failure`, and the last good rows stand.
pub struct SqlModelMaterializer {
    db: Database,
    view: String,
    sql: String,
    tables: Vec<String>,
    every: Option<Duration>,
    incremental: Option<String>,
}

impl SqlModelMaterializer {
    /// A materializer for `view`, stored in `m_<view>`, reading `tables`.
    pub fn new(
        db: Database,
        view: &str,
        sql: &str,
        tables: Vec<String>,
        materialize: &oxplow_db::models::Materialize,
    ) -> Self {
        Self {
            db,
            view: view.to_string(),
            sql: sql.to_string(),
            tables,
            every: materialize.every(),
            incremental: materialize.incremental().map(str::to_string),
        }
    }

    /// Refill the table whole; report what it now holds.
    async fn refill(&self) -> Result<Recomputed, DomainError> {
        let table = oxplow_db::models::materialized_table(&self.view);
        let sql = self.sql.clone();
        let watermark = self.incremental.clone();
        self.db
            .transaction(move |tx| {
                tx.execute_batch(&format!(
                    "DELETE FROM \"{table}\"; INSERT INTO \"{table}\" SELECT * FROM ({sql});"
                ))
                .map_err(oxplow_db::map_sql_err)?;
                held(tx, &table, watermark.as_deref(), "full")
            })
            .await
    }

    /// Append the rows past the watermark the table holds.
    async fn append(&self, column: &str) -> Result<Recomputed, DomainError> {
        let table = oxplow_db::models::materialized_table(&self.view);
        let sql = self.sql.clone();
        let column = column.to_string();
        self.db
            .transaction(move |tx| {
                let quoted = column.replace('"', "\"\"");
                let mark: Option<i64> = tx
                    .query_row(
                        &format!("SELECT max(\"{quoted}\") FROM \"{table}\""),
                        [],
                        |r| r.get(0),
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                tx.execute(
                    &format!(
                        "INSERT INTO \"{table}\" SELECT * FROM ({sql}) \
                         WHERE ?1 IS NULL OR \"{quoted}\" > ?1"
                    ),
                    [mark],
                )
                .map_err(oxplow_db::map_sql_err)?;
                held(tx, &table, Some(&column), "incremental")
            })
            .await
    }
}

/// What a materialized table holds after a recompute.
fn held(
    tx: &rusqlite::Connection,
    table: &str,
    watermark: Option<&str>,
    mode: &'static str,
) -> Result<Recomputed, DomainError> {
    let row_count: i64 = tx
        .query_row(&format!("SELECT count(*) FROM \"{table}\""), [], |r| {
            r.get(0)
        })
        .map_err(oxplow_db::map_sql_err)?;
    let watermark = match watermark {
        Some(column) => tx
            .query_row(
                &format!(
                    "SELECT max(\"{}\") FROM \"{table}\"",
                    column.replace('"', "\"\"")
                ),
                [],
                |r| r.get(0),
            )
            .map_err(oxplow_db::map_sql_err)?,
        None => None,
    };
    Ok(Recomputed {
        snapshot_id: None,
        mode: Some(mode),
        watermark,
        row_count: Some(row_count),
    })
}

#[async_trait]
impl Materializer for SqlModelMaterializer {
    fn asset(&self) -> &str {
        &self.view
    }

    fn inputs(&self) -> Vec<String> {
        self.tables.clone()
    }

    fn every(&self) -> Option<Duration> {
        self.every
    }

    /// Its compiled SELECT, hashed.
    fn definition(&self) -> Option<String> {
        use sha2::{Digest, Sha256};
        Some(hex::encode(Sha256::digest(self.sql.as_bytes())))
    }

    /// Refill whole, or append past the watermark. An append that hits
    /// the key is a failure like any other: with the watermark the key, it
    /// means the SELECT emitted one key twice, which a refill would hit
    /// too (tsk781).
    async fn recompute(&self, full: bool) -> Result<Recomputed, DomainError> {
        match self.incremental.as_deref().filter(|_| !full) {
            Some(column) => self.append(column).await,
            None => self.refill().await,
        }
    }
}

/// How long until a clocked asset is due: `every` after its last recorded
/// recompute, nothing when it never ran (or the record is unreadable), is
/// overdue, or last computed another `definition` (its SELECT changed).
async fn until_due(
    db: &Database,
    asset: &str,
    definition: Option<String>,
    every: Duration,
    now: Timestamp,
) -> Duration {
    let asset = asset.to_string();
    let last = db
        .read(move |tx| {
            use rusqlite::OptionalExtension;
            tx.query_row(
                "SELECT computed_at, definition FROM asset_state WHERE asset = ?1",
                [&asset],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
            )
            .optional()
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .ok()
        .flatten()
        .filter(|(_, recorded)| *recorded == definition)
        .and_then(|(at, _)| Timestamp::parse(&at).ok());
    let Some(last) = last else {
        return Duration::ZERO;
    };
    let age_ms = (now.unix_ms() - last.unix_ms()).max(0) as u64;
    every.saturating_sub(Duration::from_millis(age_ms))
}

/// Recompute one asset — whole when it needs it — and record it in
/// `asset_state` at `now` (the clock `until_due` reads too).
async fn recompute(db: &Database, entry: &Entry, now: &Now) {
    let m = entry.materializer.as_ref();
    let full = entry
        .needs_full
        .swap(false, std::sync::atomic::Ordering::SeqCst);
    let asset = m.asset().to_string();
    let events_to = match db
        .read(|tx| {
            tx.query_row("SELECT coalesce(max(seq), 0) FROM event_log", [], |r| {
                r.get::<_, i64>(0)
            })
            .map_err(oxplow_db::map_sql_err)
        })
        .await
    {
        Ok(seq) => seq,
        Err(error) => {
            if full {
                entry
                    .needs_full
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
            tracing::warn!(%asset, %error, "an asset couldn't read the log's position; not recomputed");
            return;
        }
    };
    let started = Instant::now();
    let definition = m.definition();
    match m.recompute(full).await {
        Ok(done) => {
            let elapsed = started.elapsed().as_millis() as i64;
            let at = now().to_string();
            let recorded = db
                .transaction(move |tx| {
                    tx.execute(
                        "INSERT INTO asset_state (asset, computed_at, events_to, snapshot_id, elapsed_ms,
                                                  mode, watermark, row_count, definition)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                         ON CONFLICT (asset) DO UPDATE SET
                            computed_at = excluded.computed_at, events_to = excluded.events_to,
                            snapshot_id = excluded.snapshot_id, elapsed_ms = excluded.elapsed_ms,
                            mode = excluded.mode, watermark = excluded.watermark,
                            row_count = excluded.row_count, definition = excluded.definition",
                        rusqlite::params![
                            asset,
                            at,
                            events_to,
                            done.snapshot_id,
                            elapsed,
                            done.mode,
                            done.watermark,
                            done.row_count,
                            definition
                        ],
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                    tx.execute("DELETE FROM asset_failure WHERE asset = ?1", [&asset])
                        .map(|_| ())
                        .map_err(oxplow_db::map_sql_err)
                })
                .await;
            if let Err(error) = recorded {
                tracing::warn!(asset = %m.asset(), %error, "recording an asset's recompute failed");
            }
        }
        Err(error) => {
            if full {
                entry
                    .needs_full
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }
            tracing::warn!(asset = %m.asset(), %error, "an asset's recompute failed; the next change tries again");
            let (asset, at, message) =
                (m.asset().to_string(), now().to_string(), error.to_string());
            let recorded = db
                .transaction(move |tx| {
                    tx.execute(
                        "INSERT INTO asset_failure (asset, failed_at, error) VALUES (?1, ?2, ?3)
                         ON CONFLICT (asset) DO UPDATE SET
                            failed_at = excluded.failed_at, error = excluded.error",
                        rusqlite::params![asset, at, message],
                    )
                    .map(|_| ())
                    .map_err(oxplow_db::map_sql_err)
                })
                .await;
            if let Err(error) = recorded {
                tracing::warn!(asset = %m.asset(), %error, "recording an asset's failure failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Counting {
        runs: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Materializer for Counting {
        fn asset(&self) -> &str {
            "counting"
        }
        fn inputs(&self) -> Vec<String> {
            vec!["task".into()]
        }
        async fn recompute(&self, _full: bool) -> Result<Recomputed, DomainError> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            Ok(Recomputed {
                snapshot_id: Some(7),
                ..Recomputed::default()
            })
        }
    }

    async fn state(db: &Database) -> Option<(i64, Option<i64>)> {
        db.read(|tx| {
            use rusqlite::OptionalExtension;
            tx.query_row(
                "SELECT events_to, snapshot_id FROM asset_state WHERE asset = 'counting'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap()
    }

    /// An asset builds once when registered, recomputes once after a burst
    /// of changes to its inputs (not once per change), ignores tables it
    /// doesn't read, and records each recompute.
    #[tokio::test]
    async fn an_asset_recomputes_once_per_quiet_burst_on_its_inputs() {
        let db = Database::in_memory();
        let runs = Arc::new(AtomicUsize::new(0));
        let assets = Assets::new(db.clone(), Duration::from_millis(50));
        assets.register(Arc::new(Counting { runs: runs.clone() }));
        let settle = || tokio::time::sleep(Duration::from_millis(200));
        settle().await;
        assert_eq!(runs.load(Ordering::SeqCst), 1, "the first build");
        assert_eq!(state(&db).await, Some((0, Some(7))));

        let task = oxplow_db::changes::Changed::inserted(["task".to_string()]);
        for _ in 0..5 {
            assets.changed(&task);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        settle().await;
        assert_eq!(
            runs.load(Ordering::SeqCst),
            2,
            "one recompute for the burst"
        );

        assets.changed(&oxplow_db::changes::Changed::inserted([
            "thread".to_string()
        ]));
        settle().await;
        assert_eq!(runs.load(Ordering::SeqCst), 2, "not one of its inputs");
    }

    /// An asset that logs when it starts and ends, and takes a while.
    struct Logged {
        name: &'static str,
        inputs: Vec<String>,
        log: Arc<std::sync::Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl Materializer for Logged {
        fn asset(&self) -> &str {
            self.name
        }
        fn inputs(&self) -> Vec<String> {
            self.inputs.clone()
        }
        async fn recompute(&self, _full: bool) -> Result<Recomputed, DomainError> {
            self.log
                .lock()
                .unwrap()
                .push(format!("{} start", self.name));
            tokio::time::sleep(Duration::from_millis(100)).await;
            self.log.lock().unwrap().push(format!("{} end", self.name));
            Ok(Recomputed::default())
        }
    }

    /// An asset that reads another asset recomputes after it, never while
    /// it is pending: its first build waits for the other's, and a change
    /// to the other's inputs reaches it once the other has recomputed.
    #[tokio::test]
    async fn an_asset_reading_another_follows_it() {
        let db = Database::in_memory();
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let assets = Assets::new(db.clone(), Duration::from_millis(30));
        assets.register(Arc::new(Logged {
            name: "cube",
            inputs: vec!["fact".into()],
            log: log.clone(),
        }));
        assets.register(Arc::new(Logged {
            name: "evidence",
            inputs: vec!["cube".into()],
            log: log.clone(),
        }));
        let settle = || tokio::time::sleep(Duration::from_millis(600));
        settle().await;
        assets.changed(&oxplow_db::changes::Changed::inserted(["fact".to_string()]));
        settle().await;
        assert_eq!(
            *log.lock().unwrap(),
            [
                "cube start",
                "cube end",
                "evidence start",
                "evidence end",
                "cube start",
                "cube end",
                "evidence start",
                "evidence end",
            ]
        );
    }

    struct Clocked {
        runs: Arc<AtomicUsize>,
        every: Duration,
        definition: Option<&'static str>,
    }

    #[async_trait]
    impl Materializer for Clocked {
        fn asset(&self) -> &str {
            "counting"
        }
        fn inputs(&self) -> Vec<String> {
            vec!["task".into()]
        }
        fn every(&self) -> Option<Duration> {
            Some(self.every)
        }
        fn definition(&self) -> Option<String> {
            self.definition.map(str::to_string)
        }
        async fn recompute(&self, _full: bool) -> Result<Recomputed, DomainError> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            Ok(Recomputed::default())
        }
    }

    /// A clock that follows tokio's (paused, in these tests): what the
    /// assets record and what they wait on agree.
    fn paused_now() -> Now {
        let (base, at) = (Timestamp::now(), tokio::time::Instant::now());
        Arc::new(move || Timestamp::from_unix_ms(base.unix_ms() + at.elapsed().as_millis() as i64))
    }

    const HOUR: Duration = Duration::from_secs(3600);
    const MINUTE: Duration = Duration::from_secs(60);

    async fn record(db: &Database, at: Timestamp, definition: Option<&'static str>) {
        let at = at.to_string();
        db.transaction(move |tx| {
            tx.execute(
                "INSERT INTO asset_state (asset, computed_at, events_to, elapsed_ms, definition)
                 VALUES ('counting', ?1, 0, 1, ?2)",
                rusqlite::params![at, definition],
            )
            .map(|_| ())
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap();
    }

    fn clocked(runs: &Arc<AtomicUsize>, definition: Option<&'static str>) -> Arc<Clocked> {
        Arc::new(Clocked {
            runs: runs.clone(),
            every: HOUR,
            definition,
        })
    }

    /// P8.B2: a clocked asset builds when it never ran, then on its clock —
    /// its inputs' changes don't recompute it.
    #[tokio::test(start_paused = true)]
    async fn a_clocked_asset_recomputes_on_its_clock_not_on_changes() {
        let db = Database::in_memory();
        let runs = Arc::new(AtomicUsize::new(0));
        let assets = Assets::new(db.clone(), Duration::from_millis(10)).with_now(paused_now());
        assets.register(clocked(&runs, None));
        tokio::time::sleep(MINUTE).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1, "the first build");
        assets.changed(&oxplow_db::changes::Changed::inserted(["task".to_string()]));
        tokio::time::sleep(10 * MINUTE).await;
        assert_eq!(
            runs.load(Ordering::SeqCst),
            1,
            "a change doesn't recompute it"
        );
        tokio::time::sleep(50 * MINUTE).await;
        assert_eq!(runs.load(Ordering::SeqCst), 2, "the clock does");
    }

    /// A restart with a fresh `computed_at` waits out the rest of the
    /// clock instead of rebuilding.
    #[tokio::test(start_paused = true)]
    async fn a_fresh_clocked_asset_waits_after_a_restart() {
        let db = Database::in_memory();
        let now = paused_now();
        record(&db, now(), None).await;
        let runs = Arc::new(AtomicUsize::new(0));
        let assets = Assets::new(db.clone(), Duration::from_millis(10)).with_now(now);
        assets.register(clocked(&runs, None));
        tokio::time::sleep(30 * MINUTE).await;
        assert_eq!(runs.load(Ordering::SeqCst), 0, "still fresh");
        tokio::time::sleep(31 * MINUTE).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1, "due, so recomputed");
    }

    /// tsk794: what a recompute records is on the same clock it waits on,
    /// so a restart 50 minutes after a build waits the other 10, not an
    /// hour.
    #[tokio::test(start_paused = true)]
    async fn a_restart_waits_only_the_rest_of_the_clock() {
        let db = Database::in_memory();
        let now = paused_now();
        let runs = Arc::new(AtomicUsize::new(0));
        let first = Assets::new(db.clone(), Duration::from_millis(10)).with_now(now.clone());
        first.register(clocked(&runs, None));
        tokio::time::sleep(MINUTE).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1, "the first build");
        tokio::time::sleep(50 * MINUTE).await;
        first.remove("counting");
        let restarted = Assets::new(db.clone(), Duration::from_millis(10)).with_now(now);
        restarted.register(clocked(&runs, None));
        tokio::time::sleep(5 * MINUTE).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1, "not due yet");
        tokio::time::sleep(6 * MINUTE).await;
        assert_eq!(
            runs.load(Ordering::SeqCst),
            2,
            "due an hour after the build"
        );
    }

    /// tsk780: a fresh record of a *different* definition (the model's
    /// SELECT was edited) doesn't hold the clock: it rebuilds at once.
    #[tokio::test(start_paused = true)]
    async fn a_clocked_asset_whose_definition_changed_rebuilds_at_once() {
        let db = Database::in_memory();
        let now = paused_now();
        record(&db, now(), Some("old")).await;
        let runs = Arc::new(AtomicUsize::new(0));
        let assets = Assets::new(db.clone(), Duration::from_millis(10)).with_now(now);
        assets.register(clocked(&runs, Some("new")));
        tokio::time::sleep(MINUTE).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1, "rebuilt for its new SQL");
        let recorded: Option<String> = db
            .read(|tx| {
                tx.query_row(
                    "SELECT definition FROM asset_state WHERE asset = 'counting'",
                    [],
                    |r| r.get(0),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(recorded.as_deref(), Some("new"));
    }

    /// Records whether each recompute was whole.
    struct Recording {
        fulls: Arc<std::sync::Mutex<Vec<bool>>>,
    }

    #[async_trait]
    impl Materializer for Recording {
        fn asset(&self) -> &str {
            "recording"
        }
        fn inputs(&self) -> Vec<String> {
            vec!["src".into()]
        }
        async fn recompute(&self, full: bool) -> Result<Recomputed, DomainError> {
            self.fulls.lock().unwrap().push(full);
            Ok(Recomputed::default())
        }
    }

    /// tsk794: the wiring end to end — a real write, the database's change
    /// hook, `Assets::changed`: an insert appends, an update forces a
    /// whole recompute.
    #[tokio::test(start_paused = true)]
    async fn a_real_update_to_an_input_forces_a_whole_recompute() {
        let db = Database::in_memory();
        db.transaction(|tx| {
            tx.execute_batch("CREATE TABLE src (id INTEGER PRIMARY KEY, v TEXT)")
                .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap();
        let mut rx = db.subscribe_changes();
        let fulls = Arc::new(std::sync::Mutex::new(Vec::new()));
        let assets = Assets::new(db.clone(), Duration::from_millis(10));
        assets.register(Arc::new(Recording {
            fulls: fulls.clone(),
        }));
        tokio::time::sleep(MINUTE).await;
        let write = |sql: &'static str| {
            let db = db.clone();
            async move {
                db.transaction(move |tx| tx.execute_batch(sql).map_err(oxplow_db::map_sql_err))
                    .await
                    .unwrap()
            }
        };
        // The assets' own records (`asset_state`) come through too: hand
        // on everything up to the write's.
        async fn deliver(
            rx: &mut tokio::sync::broadcast::Receiver<oxplow_db::changes::TablesChanged>,
            assets: &Assets,
        ) {
            loop {
                let changed = rx.recv().await.unwrap();
                assets.changed(&changed);
                if changed.contains("src") {
                    return;
                }
            }
        }
        write("INSERT INTO src (id, v) VALUES (1, 'a')").await;
        deliver(&mut rx, &assets).await;
        tokio::time::sleep(MINUTE).await;
        write("UPDATE src SET v = 'b' WHERE id = 1").await;
        deliver(&mut rx, &assets).await;
        tokio::time::sleep(MINUTE).await;
        assert_eq!(*fulls.lock().unwrap(), vec![true, false, true]);
    }

    /// A table `src` and the table of an incremental model over it, keyed
    /// and watermarked on `id`.
    async fn incremental_fixture() -> (Database, SqlModelMaterializer) {
        let db = Database::in_memory();
        db.transaction(|tx| {
            tx.execute_batch(
                "CREATE TABLE src (id INTEGER PRIMARY KEY, v INTEGER NOT NULL);
                 CREATE TABLE m_v_inc (id INTEGER, v INTEGER, PRIMARY KEY (id));",
            )
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap();
        let m = SqlModelMaterializer::new(
            db.clone(),
            "v_inc",
            "SELECT id, v FROM src WHERE v % 3 != 0",
            vec!["src".into()],
            &oxplow_db::models::Materialize::Incremental {
                incremental: "id".into(),
            },
        );
        (db, m)
    }

    async fn rows(db: &Database, sql: &'static str) -> Vec<(i64, i64)> {
        db.read(move |tx| {
            let mut st = tx.prepare(sql).map_err(oxplow_db::map_sql_err)?;
            let rows = st
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(oxplow_db::map_sql_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(oxplow_db::map_sql_err)?;
            Ok(rows)
        })
        .await
        .unwrap()
    }

    async fn write(db: &Database, sql: String) {
        db.transaction(move |tx| tx.execute_batch(&sql).map_err(oxplow_db::map_sql_err))
            .await
            .unwrap();
    }

    async fn mode(db: &Database) -> Option<String> {
        db.read(|tx| {
            use rusqlite::OptionalExtension;
            tx.query_row(
                "SELECT mode FROM asset_state WHERE asset = 'v_inc'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap()
        .flatten()
    }

    /// P8.B4: an incremental model's first build is whole, an insert
    /// appends, and a rewrite of an input (an UPDATE, as a payload expiry
    /// is) refills it whole.
    #[tokio::test]
    async fn an_incremental_model_appends_until_an_input_is_rewritten() {
        let (db, m) = incremental_fixture().await;
        let assets = Assets::new(db.clone(), Duration::from_millis(20));
        write(&db, "INSERT INTO src (v) VALUES (1), (2), (3);".into()).await;
        assets.register(Arc::new(m));
        let settle = || tokio::time::sleep(Duration::from_millis(150));
        settle().await;
        assert_eq!(mode(&db).await.as_deref(), Some("full"), "the first build");

        write(&db, "INSERT INTO src (v) VALUES (4), (5);".into()).await;
        assets.changed(&oxplow_db::changes::Changed::inserted(["src".to_string()]));
        settle().await;
        assert_eq!(mode(&db).await.as_deref(), Some("incremental"));

        write(&db, "UPDATE src SET v = 7 WHERE id = 1;".into()).await;
        let mut rewrote = oxplow_db::changes::Changed::inserted(["src".to_string()]);
        rewrote.rewrote.insert("src".into());
        assets.changed(&rewrote);
        settle().await;
        assert_eq!(
            mode(&db).await.as_deref(),
            Some("full"),
            "a rewrite refills"
        );
        assert_eq!(
            rows(&db, "SELECT id, v FROM m_v_inc ORDER BY id").await,
            rows(&db, "SELECT id, v FROM src WHERE v % 3 != 0 ORDER BY id").await
        );
    }

    /// One write to `src` in a random sequence.
    #[derive(Debug, Clone)]
    enum Op {
        Insert(i64),
        Update(usize, i64),
        Delete(usize),
    }

    fn op() -> impl proptest::strategy::Strategy<Value = Op> {
        use proptest::prelude::*;
        prop_oneof![
            4 => (0i64..100).prop_map(Op::Insert),
            1 => (0usize..50, 0i64..100).prop_map(|(i, v)| Op::Update(i, v)),
            1 => (0usize..50).prop_map(Op::Delete),
        ]
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(40))]
        /// Over any sequence of inserts, updates and deletes — a rewrite
        /// asking for a refill, as the change hook reports it — the
        /// incremental table always equals what a full refill would hold.
        #[test]
        fn incremental_equals_full(ops in proptest::collection::vec(op(), 1..30)) {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async {
                let (db, m) = incremental_fixture().await;
                m.recompute(true).await.unwrap();
                for op in ops {
                    let ids: Vec<i64> = rows(&db, "SELECT id, v FROM src ORDER BY id")
                        .await
                        .into_iter()
                        .map(|(id, _)| id)
                        .collect();
                    let rewrote = match op {
                        Op::Insert(v) => {
                            write(&db, format!("INSERT INTO src (v) VALUES ({v});")).await;
                            false
                        }
                        Op::Update(i, v) if !ids.is_empty() => {
                            let id = ids[i % ids.len()];
                            write(&db, format!("UPDATE src SET v = {v} WHERE id = {id};")).await;
                            true
                        }
                        Op::Delete(i) if !ids.is_empty() => {
                            let id = ids[i % ids.len()];
                            write(&db, format!("DELETE FROM src WHERE id = {id};")).await;
                            true
                        }
                        _ => false,
                    };
                    m.recompute(rewrote).await.unwrap();
                    assert_eq!(
                        rows(&db, "SELECT id, v FROM m_v_inc ORDER BY id").await,
                        rows(&db, "SELECT id, v FROM src WHERE v % 3 != 0 ORDER BY id").await
                    );
                }
            });
        }
    }

    /// tsk781: an append that hits the key is a failure, not a silent
    /// refill — with the watermark the key (tsk777), only SQL that emits a
    /// key twice can hit it, and a refill of the same SQL fails the same
    /// way. (The shape below, the latest row per key, is one the compiler
    /// refuses — tsk778; the materializer is driven directly.)
    #[tokio::test]
    async fn an_append_that_hits_its_key_fails() {
        let db = Database::in_memory();
        db.transaction(|tx| {
            tx.execute_batch(
                "CREATE TABLE src (id INTEGER PRIMARY KEY, k INTEGER NOT NULL);
                 CREATE TABLE m_v_latest (k INTEGER, seq INTEGER, PRIMARY KEY (k));",
            )
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap();
        let m = SqlModelMaterializer::new(
            db.clone(),
            "v_latest",
            "SELECT k, id AS seq FROM src s WHERE NOT EXISTS
               (SELECT 1 FROM src n WHERE n.k = s.k AND n.id > s.id)",
            vec!["src".into()],
            &oxplow_db::models::Materialize::Incremental {
                incremental: "seq".into(),
            },
        );
        write(&db, "INSERT INTO src (id, k) VALUES (1, 1), (2, 2);".into()).await;
        m.recompute(true).await.unwrap();
        write(&db, "INSERT INTO src (id, k) VALUES (3, 1);".into()).await;
        let err = m.recompute(false).await.unwrap_err();
        assert!(err.to_string().contains("UNIQUE"), "{err}");
        assert_eq!(
            rows(&db, "SELECT k, seq FROM m_v_latest ORDER BY k").await,
            vec![(1, 1), (2, 2)],
            "nothing appended; the last good rows stand"
        );
    }

    /// `v_asset`'s last failure for `asset`, and when.
    async fn surfaced(db: &Database, asset: &'static str) -> (Option<String>, Option<String>) {
        db.read(move |tx| {
            tx.query_row(
                "SELECT error, failed_at FROM v_asset WHERE asset = ?1",
                [asset],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap()
    }

    /// tsk781: a refill that fails — a keyed SELECT that emits one key
    /// twice — is the asset's failure in `v_asset`, not only a log line;
    /// the next success clears it.
    #[tokio::test]
    async fn a_failed_recompute_shows_in_v_asset_until_one_succeeds() {
        let db = Database::in_memory();
        write(
            &db,
            "CREATE TABLE src (id INTEGER, v INTEGER);
             CREATE TABLE m_v_dup (id INTEGER, v INTEGER, PRIMARY KEY (id));
             INSERT INTO src VALUES (1, 1), (1, 2);"
                .into(),
        )
        .await;
        let assets = Assets::new(db.clone(), Duration::from_millis(10));
        assets.register(Arc::new(SqlModelMaterializer::new(
            db.clone(),
            "v_dup",
            "SELECT id, v FROM src",
            vec!["src".into()],
            &oxplow_db::models::Materialize::Named(oxplow_db::models::MaterializePolicy::OnChange),
        )));
        tokio::time::sleep(Duration::from_millis(150)).await;
        let (error, _) = surfaced(&db, "v_dup").await;
        assert!(
            error.as_deref().is_some_and(|e| e.contains("UNIQUE")),
            "{error:?}"
        );
        write(&db, "DELETE FROM src WHERE v = 2;".into()).await;
        assets.changed(&oxplow_db::changes::Changed {
            tables: ["src".to_string()].into(),
            rewrote: ["src".to_string()].into(),
        });
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(surfaced(&db, "v_dup").await.0, None, "cleared by a success");
    }
}
