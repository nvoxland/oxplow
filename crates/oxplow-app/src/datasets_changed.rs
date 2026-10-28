//! The one change loop (P4.6, P7.B1; `.context/semantic-layer.md`
//! "Subscriptions"): the database says which tables each commit touched
//! (`Database::subscribe_changes`), and three things follow from it.
//!
//! - **Datasets.** This follows the lineage the dataset compiler recorded
//!   (`dataset_input`: a dataset reads a table through `source()` or another
//!   dataset through `ref()`) from those tables to every dataset that reads
//!   them, directly or through other datasets, stamps each one's watermark,
//!   and announces `OxplowEvent::DatasetsChanged`. A page re-runs when a
//!   dataset it read is in it; a query result carries its datasets'
//!   watermarks as `freshness`.
//! - **Metric samples.** A commit to `metric_capture` or `fact` is
//!   announced as `OxplowEvent::MetricSamplesChanged`, per stream, naming
//!   the measures of the facts that landed — the one place that event is
//!   made, so no recording site can forget it ([`CaptureListener`]).
//! - **Assets.** Each asset whose inputs it touched is marked dirty
//!   (`assets::Assets`), and recomputes once its inputs are quiet.
//!
//! A live dataset (`live_datasets`) has no table to touch: it is announced
//! with the datasets it reads through ([`live_datasets_reading`], in the loop
//! below), and after the events about the state it reads
//! ([`live_datasets_after`], which the bus asks as it posts one).
//!
//! The watermarks are in memory: they're derivable (nothing is lost by a
//! restart but "changed since boot"), and they change on every write.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock};

use oxplow_db::{Database, DatasetFreshness};
use oxplow_domain::{DomainError, Timestamp};

use crate::events::{EventBus, OxplowEvent};

/// Input (a table or a dataset's view) → the datasets that read it directly.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Lineage {
    readers: HashMap<String, Vec<String>>,
}

impl Lineage {
    pub async fn load(db: &Database) -> Result<Self, DomainError> {
        let rows: Vec<(String, String)> = db
            .read(|tx| {
                // A materialized dataset changes when its table is refilled
                // (P7.B2), not when its inputs do.
                let mut st = tx
                    .prepare(
                        "SELECT i.input, i.view FROM dataset_input i JOIN dataset m USING (view)
                          WHERE m.materialize IS NULL
                         UNION ALL
                         SELECT 'm_' || view, view FROM dataset WHERE materialize IS NOT NULL",
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = st
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map_err(oxplow_db::map_sql_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(oxplow_db::map_sql_err)?;
                Ok(rows)
            })
            .await?;
        let mut readers: HashMap<String, Vec<String>> = HashMap::new();
        for (input, view) in rows {
            readers.entry(input).or_default().push(view);
        }
        Ok(Self { readers })
    }

    /// Every dataset that reads any of `tables`, directly or through other
    /// datasets; sorted.
    pub fn affected<'a>(&self, tables: impl IntoIterator<Item = &'a String>) -> Vec<String> {
        let mut out = BTreeSet::new();
        let mut todo: Vec<&str> = tables.into_iter().map(String::as_str).collect();
        while let Some(input) = todo.pop() {
            for view in self.readers.get(input).into_iter().flatten() {
                if out.insert(view.clone()) {
                    todo.push(view);
                }
            }
        }
        out.into_iter().collect()
    }

    /// Every dataset.
    fn all(&self) -> Vec<String> {
        let all: BTreeSet<String> = self.readers.values().flatten().cloned().collect();
        all.into_iter().collect()
    }
}

/// When each dataset last changed, since boot.
#[derive(Debug, Default)]
pub struct DatasetWatermarks {
    changed_at: RwLock<HashMap<String, Timestamp>>,
}

impl DatasetWatermarks {
    fn mark(&self, datasets: &[String], at: Timestamp) {
        let mut m = self.changed_at.write().unwrap_or_else(|e| e.into_inner());
        for dataset in datasets {
            m.insert(dataset.clone(), at);
        }
    }

    /// The watermarks of `datasets` that have changed since boot.
    pub fn freshness(&self, datasets: &[String]) -> Vec<DatasetFreshness> {
        let m = self.changed_at.read().unwrap_or_else(|e| e.into_inner());
        datasets
            .iter()
            .filter_map(|dataset| {
                m.get(dataset).map(|at| DatasetFreshness {
                    dataset: dataset.clone(),
                    changed_at: at.to_text(),
                })
            })
            .collect()
    }
}

/// Where the listener has read the capture and fact tables up to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CaptureListener {
    capture: i64,
    fact: i64,
}

impl CaptureListener {
    /// Start at the tables' current ends: only what lands from now on is
    /// announced.
    pub async fn at_end(db: &Database) -> Result<Self, DomainError> {
        db.read(|tx| {
            tx.query_row(
                "SELECT (SELECT coalesce(max(id), 0) FROM metric_capture),
                        (SELECT coalesce(max(id), 0) FROM fact)",
                [],
                |r| {
                    Ok(Self {
                        capture: r.get(0)?,
                        fact: r.get(1)?,
                    })
                },
            )
            .map_err(oxplow_db::map_sql_err)
        })
        .await
    }

    /// What landed since the last call, per stream: the measures of its
    /// new facts, sorted — and an empty list (fail-open: "unknown, refresh
    /// anyway") for a stream with a new capture that recorded none.
    pub async fn landed(
        &mut self,
        db: &Database,
    ) -> Result<Vec<(oxplow_domain::StreamId, Vec<String>)>, DomainError> {
        let since = *self;
        let (rows, next) = db
            .read(move |tx| {
                let mut st = tx
                    .prepare(
                        "SELECT c.stream_id, m.key FROM fact f
                           JOIN metric_capture c ON c.id = f.capture_id
                           JOIN measure m ON m.id = f.measure_id
                          WHERE f.id > ?1
                         UNION
                         SELECT c.stream_id, m.key FROM fact_chain ch
                           JOIN metric_capture c ON c.id = ch.capture_id
                           JOIN measure m ON m.id = ch.measure_id
                          WHERE c.id > ?2
                         UNION
                         SELECT c.stream_id, m.key FROM fact_path_hold ph
                           JOIN metric_capture c ON c.id = ph.capture_id
                           JOIN measure m ON m.id = ph.measure_id
                          WHERE c.id > ?2
                         UNION
                         SELECT c.stream_id, NULL FROM metric_capture c
                          WHERE c.id > ?2
                            AND NOT EXISTS (SELECT 1 FROM fact f WHERE f.capture_id = c.id)
                            AND NOT EXISTS (SELECT 1 FROM fact_chain ch WHERE ch.capture_id = c.id)
                            AND NOT EXISTS (SELECT 1 FROM fact_path_hold ph WHERE ph.capture_id = c.id)",
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = st
                    .query_map([since.fact, since.capture], |r| {
                        Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))
                    })
                    .map_err(oxplow_db::map_sql_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(oxplow_db::map_sql_err)?;
                let next = tx
                    .query_row(
                        "SELECT (SELECT coalesce(max(id), 0) FROM metric_capture),
                                (SELECT coalesce(max(id), 0) FROM fact)",
                        [],
                        |r| {
                            Ok(Self {
                                capture: r.get(0)?,
                                fact: r.get(1)?,
                            })
                        },
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                Ok((rows, next))
            })
            .await?;
        *self = next;
        let mut by_stream: std::collections::BTreeMap<i64, BTreeSet<String>> = Default::default();
        for (stream, measure) in rows {
            let measures = by_stream.entry(stream).or_default();
            if let Some(m) = measure {
                measures.insert(m);
            }
        }
        Ok(by_stream
            .into_iter()
            .map(|(stream, measures)| {
                (
                    oxplow_domain::StreamId::new(stream),
                    measures.into_iter().collect(),
                )
            })
            .collect())
    }
}

/// What the listener heard: the tables a batch of commits touched, or —
/// when it lagged and missed some — that anything may have changed.
pub(crate) enum Changed {
    Tables(oxplow_db::changes::TablesChanged),
    Everything,
}

impl Changed {
    fn touched(&self, table: &str) -> bool {
        match self {
            Changed::Tables(tables) => tables.contains(table),
            Changed::Everything => true,
        }
    }
}

/// Act on `changed`: wake the pump for new events, re-register the
/// materialized datasets and the searchable kinds' indexes when the dataset
/// registry or `ref_kind` changed, and tell the assets what changed. The
/// datasets it changed, and whether metric samples landed.
pub(crate) async fn react(
    db: &Database,
    lineage: &mut Lineage,
    assets: &crate::assets::Assets,
    pump: &crate::event_pump::EventPump,
    changed: &Changed,
) -> (Vec<String>, bool) {
    if changed.touched("event_log") {
        pump.wake();
    }
    if changed.touched("dataset_input") || changed.touched("dataset") {
        if let Ok(fresh) = Lineage::load(db).await {
            *lineage = fresh;
        }
        if let Err(error) = assets.sync_datasets().await {
            tracing::warn!(%error, "the materialized datasets didn't resync");
        }
    }
    // A searchable resource's index follows the resources declared
    // (`resource`) and the tables behind each one's view.
    if ["resource", "dataset_input", "dataset"]
        .iter()
        .any(|t| changed.touched(t))
    {
        if let Err(error) = assets.sync_search_kinds().await {
            tracing::warn!(%error, "the searchable resources' indexes didn't resync");
        }
    }
    match changed {
        Changed::Tables(tables) => {
            assets.changed(tables);
            (
                lineage.affected(tables.tables.iter()),
                tables.contains("metric_capture") || tables.contains("fact"),
            )
        }
        Changed::Everything => {
            assets.all_changed();
            (lineage.all(), true)
        }
    }
}

/// The live datasets (`v_<name>`) that `event` can change.
pub fn live_datasets_after(event: &OxplowEvent) -> Vec<String> {
    live_following(&crate::live_datasets::Change::Event(event))
}

/// The live datasets (`v_<name>`) that read through any of `datasets`.
pub fn live_datasets_reading(datasets: &[String]) -> Vec<String> {
    live_following(&crate::live_datasets::Change::Datasets(datasets))
}

fn live_following(change: &crate::live_datasets::Change<'_>) -> Vec<String> {
    crate::live_datasets::NAMES
        .iter()
        .filter(|name| crate::live_datasets::follows(name, change))
        .map(|name| format!("v_{name}"))
        .collect()
}

/// The announcement that follows `event` on the bus: the live datasets it
/// changed, if any.
pub fn announced_after(event: &OxplowEvent) -> Option<OxplowEvent> {
    let datasets = live_datasets_after(event);
    (!datasets.is_empty()).then_some(OxplowEvent::DatasetsChanged { datasets })
}

/// Follow the database's changes — datasets, metric samples, assets, and
/// new events for the pump (a commit that logged one wakes it, whoever
/// made it) — for the life of the process.
pub fn spawn(
    db: Database,
    watermarks: Arc<DatasetWatermarks>,
    events: EventBus,
    assets: crate::assets::Assets,
    pump: Arc<crate::event_pump::EventPump>,
    page_runs: crate::pages::open_runs::PageRuns,
) {
    let mut rx = db.subscribe_changes();
    tokio::spawn(async move {
        let mut lineage = Lineage::load(&db).await.unwrap_or_else(|error| {
            tracing::warn!(%error, "dataset lineage didn't load; no dataset will be reported changed");
            Lineage::default()
        });
        if let Err(error) = assets.sync_datasets().await {
            tracing::warn!(%error, "the materialized datasets didn't register");
        }
        if let Err(error) = assets.sync_search_kinds().await {
            tracing::warn!(%error, "the searchable ref kinds' indexes didn't register");
        }
        let mut captures = CaptureListener::at_end(&db).await.unwrap_or_else(|error| {
            tracing::warn!(%error, "the capture listener starts from the beginning");
            CaptureListener::default()
        });
        loop {
            let changed = match rx.recv().await {
                Ok(tables) => Changed::Tables(tables),
                // Missed some: anything may have changed — the registry
                // too, whose change re-registers assets (tsk854).
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => Changed::Everything,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            };
            let (mut datasets, samples) = react(&db, &mut lineage, &assets, &pump, &changed).await;
            datasets.extend(live_datasets_reading(&datasets));
            if matches!(changed, Changed::Everything) {
                page_runs.notify_everything();
            } else {
                page_runs.notify_datasets(&datasets);
            }
            if !datasets.is_empty() {
                watermarks.mark(&datasets, Timestamp::now());
                events.emit(OxplowEvent::DatasetsChanged { datasets });
            }
            if samples {
                match captures.landed(&db).await {
                    Ok(landed) => {
                        for (stream_id, measures) in landed {
                            page_runs.notify_measures(&measures);
                            events.emit(OxplowEvent::MetricSamplesChanged {
                                stream_id,
                                measures,
                            });
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "reading what metric samples landed failed")
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P4.6 (tsk491): a work-item edit changes the datasets that read
    /// `work_item` and the ones that read those — no others.
    #[tokio::test]
    async fn a_table_change_reaches_its_datasets_and_their_readers_only() {
        use oxplow_tasks::TaskStore as _;
        let f = crate::test_fixtures::services_with_task_effort().await;
        let lineage = Lineage::load(&f.svc.db).await.unwrap();
        let changed = lineage.affected(&["work_item".to_string()]);
        assert!(changed.contains(&"v_work_item".to_string()), "{changed:?}");
        assert!(!changed.contains(&"v_snapshot".to_string()), "{changed:?}");
        // Transitive: a dataset reading `v_test_run` changes with its inputs.
        let through = lineage.affected(&["metric_capture".to_string()]);
        assert!(through.contains(&"v_test_run".to_string()));
        assert!(
            through.contains(&"v_oxplow_bundled_claim_status".to_string()),
            "oxplow-bundled's claim status reads v_test_run"
        );

        // End to end: a task edit is announced for v_work_item, with a watermark.
        let watermarks = Arc::new(DatasetWatermarks::default());
        let mut rx = f.svc.events.subscribe_ui();
        spawn(
            f.svc.db.clone(),
            watermarks.clone(),
            f.svc.events.clone(),
            crate::assets::Assets::new(f.svc.db.clone(), crate::assets::COALESCE),
            f.svc.event_pump.clone(),
            crate::pages::open_runs::PageRuns::default(),
        );
        tokio::task::yield_now().await;
        let mut task = f.svc.task_store.get(f.task).await.unwrap().unwrap();
        task.title = "renamed".into();
        f.svc.task_store.update(&task).await.unwrap();
        crate::test_fixtures::restate_task(&f.svc, f.task).await;
        let datasets = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Ok(OxplowEvent::DatasetsChanged { datasets }) = rx.recv().await {
                    if datasets.contains(&"v_work_item".to_string()) {
                        return datasets;
                    }
                }
            }
        })
        .await
        .expect("DatasetsChanged for v_work_item");
        assert!(
            !datasets.contains(&"v_snapshot".to_string()),
            "{datasets:?}"
        );
        assert_eq!(watermarks.freshness(&["v_work_item".to_string()]).len(), 1);
        // A query's result carries its datasets' watermarks.
        let out = crate::sql_gateway::SqlGateway::new(f.svc.db.clone())
            .with_watermarks(watermarks.clone())
            .query_sql("SELECT count(*) FROM v_work_item", vec![], None)
            .await
            .unwrap();
        assert_eq!(
            out.freshness
                .iter()
                .map(|m| m.dataset.as_str())
                .collect::<Vec<_>>(),
            vec!["v_work_item"]
        );
        assert!(watermarks.freshness(&["v_snapshot".to_string()]).is_empty());
    }

    /// P7.B1: what landed in the capture and fact tables is announced per
    /// stream, naming the measures of its facts (sorted, once each); a
    /// capture with no facts is announced fail-open (no measures). Only
    /// what landed since the last read counts.
    #[tokio::test]
    async fn the_listener_names_the_measures_that_landed_per_stream() {
        let f = crate::test_fixtures::services_with_effort().await;
        let facts = &f.svc.fact_store;
        let mut listener = CaptureListener::at_end(&f.svc.db).await.unwrap();
        let measure = |key: &'static str| async move {
            facts
                .upsert_measure(oxplow_db::NewMeasure::new(key, key))
                .await
                .unwrap()
        };
        let (a, b) = (measure("acme.alpha").await, measure("acme.beta").await);
        facts
            .record_facts(
                oxplow_db::NewMetricCapture::done(1, "p", "s"),
                vec![
                    oxplow_db::NewFact::new(b, 1.0),
                    oxplow_db::NewFact::new(a, 2.0),
                    oxplow_db::NewFact::new(b, 3.0),
                ],
            )
            .await
            .unwrap();
        assert_eq!(
            listener.landed(&f.svc.db).await.unwrap(),
            vec![(
                oxplow_domain::StreamId::new(1),
                vec!["acme.alpha".to_string(), "acme.beta".to_string()]
            )]
        );
        assert!(listener.landed(&f.svc.db).await.unwrap().is_empty());
        facts
            .record_facts(oxplow_db::NewMetricCapture::done(1, "p2", "s"), Vec::new())
            .await
            .unwrap();
        assert_eq!(
            listener.landed(&f.svc.db).await.unwrap(),
            vec![(oxplow_domain::StreamId::new(1), Vec::new())]
        );

        // A scan that repeats the one before stores no fact of its own, but
        // still names the measure it holds.
        facts
            .record_facts(
                oxplow_db::NewMetricCapture::done(1, "p", "s"),
                vec![
                    oxplow_db::NewFact::new(b, 1.0),
                    oxplow_db::NewFact::new(a, 2.0),
                    oxplow_db::NewFact::new(b, 3.0),
                ],
            )
            .await
            .unwrap();
        assert_eq!(
            listener.landed(&f.svc.db).await.unwrap(),
            vec![(
                oxplow_domain::StreamId::new(1),
                vec!["acme.alpha".to_string(), "acme.beta".to_string()]
            )]
        );
    }

    /// P7.B1: a recorded capture is announced by the change loop and folded
    /// into the cube by the asset runner — no recording site emits anything.
    #[tokio::test]
    async fn a_recorded_capture_is_announced_and_folded_without_a_bus_event() {
        let f = crate::test_fixtures::services_with_effort().await;
        let assets =
            crate::assets::Assets::new(f.svc.db.clone(), std::time::Duration::from_millis(20));
        let mut rx = f.svc.events.subscribe_ui();
        spawn(
            f.svc.db.clone(),
            Arc::new(DatasetWatermarks::default()),
            f.svc.events.clone(),
            assets.clone(),
            f.svc.event_pump.clone(),
            crate::pages::open_runs::PageRuns::default(),
        );
        assets.register(Arc::new(crate::metric_cube::MetricCubeBuilder::new(
            (*f.svc.fact_store).clone(),
        )));
        let computed_at = || async {
            f.svc
                .db
                .read(|tx| {
                    use rusqlite::OptionalExtension;
                    tx.query_row(
                        "SELECT computed_at FROM asset_state WHERE asset = 'metric_cube'",
                        [],
                        |r| r.get::<_, String>(0),
                    )
                    .optional()
                    .map_err(oxplow_db::map_sql_err)
                })
                .await
                .unwrap()
        };
        let until = |pred: Box<dyn Fn(Option<String>) -> bool>| async move {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    let at = computed_at().await;
                    if pred(at.clone()) {
                        return at;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("the cube recorded a recompute")
        };
        let first = until(Box::new(|at| at.is_some())).await;
        let m = f
            .svc
            .fact_store
            .upsert_measure(oxplow_db::NewMeasure::new("acme.gamma", "acme.gamma"))
            .await
            .unwrap();
        f.svc
            .fact_store
            .record_facts(
                oxplow_db::NewMetricCapture::done(1, "p", "s"),
                vec![oxplow_db::NewFact::new(m, 1.0)],
            )
            .await
            .unwrap();
        let measures = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Ok(OxplowEvent::MetricSamplesChanged { measures, .. }) = rx.recv().await {
                    return measures;
                }
            }
        })
        .await
        .expect("MetricSamplesChanged from the change loop");
        assert_eq!(measures, vec!["acme.gamma".to_string()]);
        let first_c = first.clone();
        until(Box::new(move |at| at.is_some() && at != first_c)).await;
    }

    /// P7.B2: an on-change dataset recomputes once after a burst of writes
    /// to its inputs, and the change loop announces it — and a dataset
    /// reading it — when its table is refilled, not when its inputs move.
    #[tokio::test]
    async fn a_materialized_dataset_recomputes_once_per_burst_and_announces_its_readers() {
        use oxplow_db::datasets::{
            DatasetDecl, DatasetField, DatasetSource, ExtensionDatasets, Materialize,
        };
        use oxplow_tasks::TaskStore as _;
        let f = crate::test_fixtures::services_with_task_effort().await;
        let dataset =
            |name: &str, sql: &str, columns: &[(&str, &str)], on_change: bool| DatasetSource {
                decl: DatasetDecl {
                    name: name.into(),
                    version: 1,
                    description: format!("{name}."),
                    fields: columns
                        .iter()
                        .map(|(n, t)| DatasetField {
                            name: (*n).into(),
                            sql_type: (*t).into(),
                            doc: "d".into(),
                            ai_context: None,
                        })
                        .collect(),
                    relationships: vec![],
                    key: vec![],
                    tests: Vec::new(),
                    deprecated: Vec::new(),
                    materialize: on_change.then_some(Materialize::ON_CHANGE),
                    ai_context: None,
                },
                file: format!("datasets/{name}.sql"),
                sql: sql.into(),
                twin: None,
            };
        let errors = f
            .svc
            .db
            .compile_extension_datasets(vec![ExtensionDatasets {
                extension: "acme".into(),
                sources: vec![
                    dataset(
                        "titles",
                        "SELECT uri AS id, title FROM ref('work_item')",
                        &[("id", "TEXT"), ("title", "TEXT")],
                        true,
                    ),
                    dataset(
                        "title_count",
                        "SELECT count(*) AS n FROM ref('titles')",
                        &[("n", "")],
                        false,
                    ),
                ],
                config: None,
            }])
            .await
            .unwrap();
        assert_eq!(errors["acme"], Vec::<String>::new());

        // A burst is writes closer together than this; it's wide enough
        // that a loaded machine still writes the five below as one.
        let coalesce = std::time::Duration::from_millis(400);
        let assets = crate::assets::Assets::new(f.svc.db.clone(), coalesce);
        let mut rx = f.svc.events.subscribe_ui();
        spawn(
            f.svc.db.clone(),
            Arc::new(DatasetWatermarks::default()),
            f.svc.events.clone(),
            assets,
            f.svc.event_pump.clone(),
            crate::pages::open_runs::PageRuns::default(),
        );
        let titles = || async {
            f.svc
                .db
                .read(|tx| {
                    let mut st = tx
                        .prepare("SELECT title FROM v_acme_titles ORDER BY id")
                        .map_err(oxplow_db::map_sql_err)?;
                    let rows = st
                        .query_map([], |r| r.get::<_, String>(0))
                        .map_err(oxplow_db::map_sql_err)?
                        .collect::<rusqlite::Result<Vec<_>>>()
                        .map_err(oxplow_db::map_sql_err)?;
                    Ok(rows)
                })
                .await
                .unwrap()
        };
        let announced = |rx: &mut tokio::sync::broadcast::Receiver<OxplowEvent>| {
            let mut n = 0;
            let mut readers = false;
            while let Ok(e) = rx.try_recv() {
                if let OxplowEvent::DatasetsChanged { datasets } = e {
                    if datasets.contains(&"v_acme_titles".to_string()) {
                        n += 1;
                        readers |= datasets.contains(&"v_acme_title_count".to_string());
                    }
                }
            }
            (n, readers)
        };
        // Until `titles` reads `want`, up to ten seconds.
        let until_titles = |want: &'static str| async move {
            for _ in 0..200 {
                if titles().await == vec![want.to_string()] {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
            panic!("v_acme_titles never read {want}: {:?}", titles().await);
        };
        // Its first build fills it.
        until_titles("t").await;
        tokio::time::sleep(coalesce * 2).await;
        let _ = announced(&mut rx);

        let mut task = f.svc.task_store.get(f.task).await.unwrap().unwrap();
        for i in 0..5 {
            task.title = format!("renamed {i}");
            f.svc.task_store.update(&task).await.unwrap();
            crate::test_fixtures::restate_task(&f.svc, f.task).await;
        }
        until_titles("renamed 4").await;
        // Long enough for a second refill to have shown, had there been one.
        tokio::time::sleep(coalesce * 2).await;
        assert_eq!(
            announced(&mut rx),
            (1, true),
            "one refill, its reader with it"
        );
    }

    /// P7 review (tsk727): a contract change on an unchanged query (its
    /// input's column changed type underneath it) recreates the dataset's
    /// table — empty — and the dataset is refilled, not left empty until an
    /// input moves.
    #[tokio::test]
    async fn a_contract_change_alone_refills_a_materialized_dataset() {
        use oxplow_db::datasets::{
            DatasetDecl, DatasetField, DatasetSource, ExtensionDatasets, Materialize,
        };
        let f = crate::test_fixtures::services_with_task_effort().await;
        let dataset = |name: &str, version: u32, sql: &str, title_type: &str, on_change: bool| {
            DatasetSource {
                decl: DatasetDecl {
                    name: name.into(),
                    version,
                    description: format!("{name}."),
                    fields: [("id", "TEXT"), ("title", title_type)]
                        .iter()
                        .map(|(n, t)| DatasetField {
                            name: (*n).into(),
                            sql_type: (*t).into(),
                            doc: "d".into(),
                            ai_context: None,
                        })
                        .collect(),
                    relationships: vec![],
                    key: vec![],
                    tests: Vec::new(),
                    deprecated: Vec::new(),
                    materialize: on_change.then_some(Materialize::ON_CHANGE),
                    ai_context: None,
                },
                file: format!("datasets/{name}.sql"),
                sql: sql.into(),
                twin: None,
            }
        };
        // `titles` reads `base`; v2 of `base` changes `title`'s type, so
        // `titles`' contract changes with its SELECT untouched.
        let compile = |version: u32, base_sql: &str, title_type: &str| {
            let db = f.svc.db.clone();
            let sources = vec![
                dataset("base", version, base_sql, title_type, false),
                dataset(
                    "titles",
                    version,
                    "SELECT id, title FROM ref('base')",
                    title_type,
                    true,
                ),
            ];
            async move {
                let errors = db
                    .compile_extension_datasets(vec![ExtensionDatasets {
                        extension: "acme".into(),
                        sources,
                        config: None,
                    }])
                    .await
                    .unwrap();
                assert_eq!(errors["acme"], Vec::<String>::new());
            }
        };
        let rows = || async {
            f.svc
                .db
                .read(|tx| {
                    tx.query_row("SELECT count(*) FROM v_acme_titles", [], |r| {
                        r.get::<_, i64>(0)
                    })
                    .map_err(oxplow_db::map_sql_err)
                })
                .await
                .unwrap()
        };
        compile(1, "SELECT uri AS id, title FROM ref('work_item')", "TEXT").await;
        let assets =
            crate::assets::Assets::new(f.svc.db.clone(), std::time::Duration::from_millis(50));
        assets.sync_datasets().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(rows().await, 1);

        compile(
            2,
            "SELECT uri AS id, title || '' AS title FROM ref('work_item')",
            "",
        )
        .await;
        assets.sync_datasets().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(rows().await, 1, "the recreated table is refilled");
    }

    /// P7.B1: the change loop is the one place `MetricSamplesChanged` is
    /// made — a recording site that emitted its own could drift from what
    /// landed.
    #[test]
    fn only_the_change_loop_makes_metric_samples_changed() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut offenders = Vec::new();
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs")
                    && !path.ends_with("datasets_changed.rs")
                {
                    let text = std::fs::read_to_string(&path).unwrap();
                    if text.contains("emit(OxplowEvent::MetricSamplesChanged") {
                        offenders.push(path.display().to_string());
                    }
                }
            }
        }
        assert_eq!(offenders, Vec::<String>::new());
    }

    /// P8.A10: telemetry and agent activity reach the renderer as their
    /// datasets changing — a recorded use, a page visit — not as bespoke
    /// pushes.
    #[tokio::test]
    async fn telemetry_writes_name_their_datasets() {
        use oxplow_db::PageVisitStore as _;
        let f = crate::test_fixtures::services_with_effort().await;
        let mut rx = f.svc.events.subscribe_ui();
        spawn(
            f.svc.db.clone(),
            Arc::new(DatasetWatermarks::default()),
            f.svc.events.clone(),
            crate::assets::Assets::new(f.svc.db.clone(), crate::assets::COALESCE),
            f.svc.event_pump.clone(),
            crate::pages::open_runs::PageRuns::default(),
        );
        tokio::task::yield_now().await;
        f.svc
            .usage_store
            .record("wiki", serde_json::json!({ "slug": "home" }))
            .await
            .unwrap();
        f.svc
            .page_visit_store
            .record("wiki", "oxplow:oxplow/wiki?slug=home", None, None, None)
            .await
            .unwrap();
        let mut want = vec!["v_usage_event", "v_page_visit"];
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !want.is_empty() {
                if let Ok(OxplowEvent::DatasetsChanged { datasets }) = rx.recv().await {
                    want.retain(|m| !datasets.iter().any(|n| n == m));
                }
            }
        })
        .await
        .expect("DatasetsChanged for v_usage_event and v_page_visit");
    }
}
