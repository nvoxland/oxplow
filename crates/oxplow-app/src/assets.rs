//! Assets (P7.B1, `.context/semantic-layer.md` "Assets"): derived data
//! whose inputs are tables, recomputed when a commit touches one.
//!
//! Two spine mechanisms, chosen by what a computation reads
//! (`target-architecture.md` §8.2): inputs that are **tables** make an
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
    /// The tables whose commits make it stale.
    fn inputs(&self) -> Vec<String>;
    /// Recomputed on this clock instead of on its inputs' changes.
    fn every(&self) -> Option<Duration> {
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

/// The registered assets. Cloning shares them.
#[derive(Clone)]
pub struct Assets {
    db: Database,
    coalesce: Duration,
    entries: Arc<std::sync::RwLock<Vec<Arc<Entry>>>>,
    /// The materialized models registered from the model registry
    /// ([`Assets::sync_models`]), by view.
    models: Arc<tokio::sync::Mutex<BTreeMap<String, ModelAsset>>>,
}

impl Assets {
    pub fn new(db: Database, coalesce: Duration) -> Self {
        Self {
            db,
            coalesce,
            entries: Arc::default(),
            models: Arc::default(),
        }
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
        });
        self.entries
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .push(entry.clone());
        let (db, coalesce, looping) = (self.db.clone(), self.coalesce, entry.clone());
        let task = tokio::spawn(async move {
            let entry = looping;
            if let Some(every) = entry.materializer.every() {
                let mut wait = until_due(&db, entry.materializer.asset(), every).await;
                loop {
                    tokio::time::sleep(wait).await;
                    recompute(&db, &entry).await;
                    wait = every;
                }
            }
            recompute(&db, &entry).await;
            loop {
                entry.dirty.notified().await;
                // Wait until its inputs have been quiet for the window.
                while tokio::time::timeout(coalesce, entry.dirty.notified())
                    .await
                    .is_ok()
                {}
                recompute(&db, &entry).await;
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
            self.register(Arc::new(SqlModelMaterializer {
                db: self.db.clone(),
                view: view.clone(),
                sql: model.sql.clone(),
                tables: model.tables.clone(),
                every: oxplow_db::models::recorded_materialize(&model.materialize)
                    .and_then(|m| m.every()),
                incremental: oxplow_db::models::recorded_materialize(&model.materialize)
                    .and_then(|m| m.incremental().map(str::to_string)),
            }));
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
                entry.dirty.notify_one();
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
            entry.dirty.notify_one();
        }
    }
}

/// The registry's materialized models, with the tables each reads: its
/// inputs followed through live models to tables — and to a materialized
/// model's own table, which is where that one's changes come from.
async fn materialized_models(db: &Database) -> Result<BTreeMap<String, ModelAsset>, DomainError> {
    db.read(|tx| {
        let mut st = tx
            .prepare(
                "SELECT m.view, m.sql, m.materialize, c.columns_json FROM model m
                 LEFT JOIN model_contract c ON c.view = m.view AND c.version = m.version",
            )
            .map_err(oxplow_db::map_sql_err)?;
        let models: BTreeMap<String, (String, Option<String>, Option<String>)> = st
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
        let mut out = BTreeMap::new();
        for (view, (sql, materialize, contract)) in &models {
            if materialize.is_none() {
                continue;
            }
            let mut tables = BTreeSet::new();
            let mut seen = BTreeSet::new();
            let mut todo: Vec<&String> = inputs.get(view).into_iter().flatten().collect();
            while let Some(input) = todo.pop() {
                if !seen.insert(input.clone()) {
                    continue;
                }
                match models.get(input) {
                    Some((_, Some(_), _)) => {
                        tables.insert(oxplow_db::models::materialized_table(input));
                    }
                    Some((_, None, _)) => todo.extend(inputs.get(input).into_iter().flatten()),
                    None => {
                        tables.insert(input.clone());
                    }
                }
            }
            out.insert(
                view.clone(),
                ModelAsset {
                    sql: sql.clone(),
                    contract: contract.clone(),
                    tables: tables.into_iter().collect(),
                    materialize: materialize.clone().unwrap_or_default(),
                },
            );
        }
        Ok(out)
    })
    .await
}

/// A materialized model (P7.B2): its table refilled, whole, from its
/// SELECT in one transaction — when an input changes, or on its `every:`
/// clock (P8.B2). An incremental one (P8.B4) appends the rows past its
/// watermark instead, unless a refill is asked for; its primary key (the
/// model's key) turns a row that should already have been there into a
/// constraint error, and it refills instead.
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

    async fn recompute(&self, full: bool) -> Result<Recomputed, DomainError> {
        let Some(column) = self.incremental.as_deref().filter(|_| !full) else {
            return self.refill().await;
        };
        match self.append(column).await {
            Ok(done) => Ok(done),
            // A row past the watermark that collides with one it holds:
            // the output isn't monotone in the watermark after all.
            Err(error) => {
                tracing::warn!(view = %self.view, %error, "an incremental append failed; refilling whole");
                self.refill().await
            }
        }
    }
}

/// How long until a clocked asset is due: `every` after its last recorded
/// recompute, nothing when it never ran (or the record is unreadable) or
/// is overdue.
async fn until_due(db: &Database, asset: &str, every: Duration) -> Duration {
    let asset = asset.to_string();
    let last = db
        .read(move |tx| {
            use rusqlite::OptionalExtension;
            tx.query_row(
                "SELECT computed_at FROM asset_state WHERE asset = ?1",
                [&asset],
                |r| r.get::<_, String>(0),
            )
            .optional()
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .ok()
        .flatten()
        .and_then(|at| Timestamp::parse(&at).ok());
    let Some(last) = last else {
        return Duration::ZERO;
    };
    let age_ms = (Timestamp::now().unix_ms() - last.unix_ms()).max(0) as u64;
    every.saturating_sub(Duration::from_millis(age_ms))
}

/// Recompute one asset — whole when it needs it — and record it in
/// `asset_state`.
async fn recompute(db: &Database, entry: &Entry) {
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
    match m.recompute(full).await {
        Ok(done) => {
            let elapsed = started.elapsed().as_millis() as i64;
            let at = Timestamp::now().to_string();
            let recorded = db
                .transaction(move |tx| {
                    tx.execute(
                        "INSERT INTO asset_state (asset, computed_at, events_to, snapshot_id, elapsed_ms,
                                                  mode, watermark, row_count)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                         ON CONFLICT (asset) DO UPDATE SET
                            computed_at = excluded.computed_at, events_to = excluded.events_to,
                            snapshot_id = excluded.snapshot_id, elapsed_ms = excluded.elapsed_ms,
                            mode = excluded.mode, watermark = excluded.watermark,
                            row_count = excluded.row_count",
                        rusqlite::params![
                            asset,
                            at,
                            events_to,
                            done.snapshot_id,
                            elapsed,
                            done.mode,
                            done.watermark,
                            done.row_count
                        ],
                    )
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

    struct Clocked {
        runs: Arc<AtomicUsize>,
        every: Duration,
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
        async fn recompute(&self, _full: bool) -> Result<Recomputed, DomainError> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            Ok(Recomputed::default())
        }
    }

    /// P8.B2: a clocked asset builds when it never ran, then on its clock —
    /// its inputs' changes don't recompute it.
    #[tokio::test]
    async fn a_clocked_asset_recomputes_on_its_clock_not_on_changes() {
        let db = Database::in_memory();
        let runs = Arc::new(AtomicUsize::new(0));
        let assets = Assets::new(db.clone(), Duration::from_millis(10));
        assets.register(Arc::new(Clocked {
            runs: runs.clone(),
            every: Duration::from_millis(400),
        }));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1, "the first build");
        assets.changed(&oxplow_db::changes::Changed::inserted(["task".to_string()]));
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            runs.load(Ordering::SeqCst),
            1,
            "a change doesn't recompute it"
        );
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 2, "the clock does");
    }

    /// A restart with a fresh `computed_at` waits out the rest of the
    /// clock instead of rebuilding.
    #[tokio::test]
    async fn a_fresh_clocked_asset_waits_after_a_restart() {
        let db = Database::in_memory();
        let at = Timestamp::now().to_string();
        db.transaction(move |tx| {
            tx.execute(
                "INSERT INTO asset_state (asset, computed_at, events_to, elapsed_ms)
                 VALUES ('counting', ?1, 0, 1)",
                [&at],
            )
            .map(|_| ())
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap();
        let runs = Arc::new(AtomicUsize::new(0));
        let assets = Assets::new(db.clone(), Duration::from_millis(10));
        assets.register(Arc::new(Clocked {
            runs: runs.clone(),
            every: Duration::from_millis(500),
        }));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 0, "still fresh");
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(runs.load(Ordering::SeqCst), 1, "due, so recomputed");
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

    /// Output that isn't monotone in the watermark — the latest row per
    /// key, which a new row *replaces* — hits the primary key on append,
    /// and the model refills whole instead of failing. (A row that appears
    /// below the watermark is the other non-monotone case; nothing at run
    /// time can see it, which is why `plugin test` checks every
    /// incremental model against a full refill.)
    #[tokio::test]
    async fn a_replaced_row_refills_instead_of_failing() {
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
        let done = m.recompute(false).await.unwrap();
        assert_eq!(done.mode, Some("full"));
        assert_eq!(
            rows(&db, "SELECT k, seq FROM m_v_latest ORDER BY k").await,
            vec![(1, 3), (2, 2)]
        );
    }
}
