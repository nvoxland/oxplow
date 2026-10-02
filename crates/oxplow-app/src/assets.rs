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
}

/// One asset: its name, the tables it reads, and how to recompute it.
#[async_trait]
pub trait Materializer: Send + Sync {
    /// `metric_cube`, or a materialized model's view.
    fn asset(&self) -> &str;
    /// The tables whose commits make it stale.
    fn inputs(&self) -> Vec<String>;
    async fn recompute(&self) -> Result<Recomputed, DomainError>;
}

struct Entry {
    materializer: Arc<dyn Materializer>,
    inputs: BTreeSet<String>,
    dirty: Arc<Notify>,
    /// Its recompute loop, stopped when the asset goes.
    task: std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

/// A materialized model as the registry has it: its compiled SELECT and
/// the tables it reads, transitively.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ModelAsset {
    sql: String,
    tables: Vec<String>,
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
            task: std::sync::Mutex::new(None),
        });
        self.entries
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .push(entry.clone());
        let (db, coalesce, looping) = (self.db.clone(), self.coalesce, entry.clone());
        let task = tokio::spawn(async move {
            let entry = looping;
            recompute(&db, entry.materializer.as_ref()).await;
            loop {
                entry.dirty.notified().await;
                // Wait until its inputs have been quiet for the window.
                while tokio::time::timeout(coalesce, entry.dirty.notified())
                    .await
                    .is_ok()
                {}
                recompute(&db, entry.materializer.as_ref()).await;
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
            }));
            have.insert(view, model);
        }
        Ok(())
    }

    /// `tables` changed: mark every asset reading one dirty.
    pub fn changed(&self, tables: &BTreeSet<String>) {
        for entry in self
            .entries
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
        {
            if entry.inputs.iter().any(|t| tables.contains(t)) {
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
            .prepare("SELECT view, sql, materialize FROM model")
            .map_err(oxplow_db::map_sql_err)?;
        let models: BTreeMap<String, (String, Option<String>)> = st
            .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))
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
        for (view, (sql, materialize)) in &models {
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
                    Some((_, Some(_))) => {
                        tables.insert(oxplow_db::models::materialized_table(input));
                    }
                    Some((_, None)) => todo.extend(inputs.get(input).into_iter().flatten()),
                    None => {
                        tables.insert(input.clone());
                    }
                }
            }
            out.insert(
                view.clone(),
                ModelAsset {
                    sql: sql.clone(),
                    tables: tables.into_iter().collect(),
                },
            );
        }
        Ok(out)
    })
    .await
}

/// A `materialize: on_change` model (P7.B2): its table refilled, whole,
/// from its SELECT in one transaction.
pub struct SqlModelMaterializer {
    db: Database,
    view: String,
    sql: String,
    tables: Vec<String>,
}

#[async_trait]
impl Materializer for SqlModelMaterializer {
    fn asset(&self) -> &str {
        &self.view
    }

    fn inputs(&self) -> Vec<String> {
        self.tables.clone()
    }

    async fn recompute(&self) -> Result<Recomputed, DomainError> {
        let table = oxplow_db::models::materialized_table(&self.view);
        let refill = format!(
            "DELETE FROM \"{table}\"; INSERT INTO \"{table}\" SELECT * FROM ({});",
            self.sql
        );
        self.db
            .transaction(move |tx| tx.execute_batch(&refill).map_err(oxplow_db::map_sql_err))
            .await?;
        Ok(Recomputed::default())
    }
}

/// Recompute one asset and record it in `asset_state`.
async fn recompute(db: &Database, m: &dyn Materializer) {
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
            tracing::warn!(%asset, %error, "an asset couldn't read the log's position; not recomputed");
            return;
        }
    };
    let started = Instant::now();
    match m.recompute().await {
        Ok(done) => {
            let elapsed = started.elapsed().as_millis() as i64;
            let at = Timestamp::now().to_string();
            let recorded = db
                .transaction(move |tx| {
                    tx.execute(
                        "INSERT INTO asset_state (asset, computed_at, events_to, snapshot_id, elapsed_ms)
                         VALUES (?1, ?2, ?3, ?4, ?5)
                         ON CONFLICT (asset) DO UPDATE SET
                            computed_at = excluded.computed_at, events_to = excluded.events_to,
                            snapshot_id = excluded.snapshot_id, elapsed_ms = excluded.elapsed_ms",
                        rusqlite::params![asset, at, events_to, done.snapshot_id, elapsed],
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
        async fn recompute(&self) -> Result<Recomputed, DomainError> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            Ok(Recomputed {
                snapshot_id: Some(7),
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

        let task: BTreeSet<String> = ["task".to_string()].into();
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

        assets.changed(&["thread".to_string()].into());
        settle().await;
        assert_eq!(runs.load(Ordering::SeqCst), 2, "not one of its inputs");
    }
}
