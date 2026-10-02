//! The one change loop (P4.6, P7.B1; `.context/semantic-layer.md`
//! "Subscriptions"): the database says which tables each commit touched
//! (`Database::subscribe_changes`), and three things follow from it.
//!
//! - **Models.** This follows the lineage the model compiler recorded
//!   (`model_input`: a model reads a table through `source()` or another
//!   model through `ref()`) from those tables to every model that reads
//!   them, directly or through other models, stamps each one's watermark,
//!   and announces `OxplowEvent::ModelsChanged`. A lens re-runs when a
//!   model it read is in it; a query result carries its models'
//!   watermarks as `freshness`.
//! - **Metric samples.** A commit to `metric_capture` or `fact` is
//!   announced as `OxplowEvent::MetricSamplesChanged`, per stream, naming
//!   the measures of the facts that landed — the one place that event is
//!   made, so no recording site can forget it ([`CaptureListener`]).
//! - **Assets.** Each asset whose inputs it touched is marked dirty
//!   (`assets::Assets`), and recomputes once its inputs are quiet.
//!
//! The watermarks are in memory: they're derivable (nothing is lost by a
//! restart but "changed since boot"), and they change on every write.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock};

use oxplow_db::{Database, ModelFreshness};
use oxplow_domain::{DomainError, Timestamp};

use crate::events::{EventBus, OxplowEvent};

/// Input (a table or a model's view) → the models that read it directly.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Lineage {
    readers: HashMap<String, Vec<String>>,
}

impl Lineage {
    pub async fn load(db: &Database) -> Result<Self, DomainError> {
        let rows: Vec<(String, String)> = db
            .read(|tx| {
                // A materialized model changes when its table is refilled
                // (P7.B2), not when its inputs do.
                let mut st = tx
                    .prepare(
                        "SELECT i.input, i.view FROM model_input i JOIN model m USING (view)
                          WHERE m.materialize IS NULL
                         UNION ALL
                         SELECT 'm_' || view, view FROM model WHERE materialize IS NOT NULL",
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

    /// Every model that reads any of `tables`, directly or through other
    /// models; sorted.
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

    /// Every model.
    fn all(&self) -> Vec<String> {
        let all: BTreeSet<String> = self.readers.values().flatten().cloned().collect();
        all.into_iter().collect()
    }
}

/// When each model last changed, since boot.
#[derive(Debug, Default)]
pub struct ModelWatermarks {
    changed_at: RwLock<HashMap<String, Timestamp>>,
}

impl ModelWatermarks {
    fn mark(&self, models: &[String], at: Timestamp) {
        let mut m = self.changed_at.write().unwrap_or_else(|e| e.into_inner());
        for model in models {
            m.insert(model.clone(), at);
        }
    }

    /// The watermarks of `models` that have changed since boot.
    pub fn freshness(&self, models: &[String]) -> Vec<ModelFreshness> {
        let m = self.changed_at.read().unwrap_or_else(|e| e.into_inner());
        models
            .iter()
            .filter_map(|model| {
                m.get(model).map(|at| ModelFreshness {
                    model: model.clone(),
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
                         SELECT c.stream_id, NULL FROM metric_capture c
                          WHERE c.id > ?2
                            AND NOT EXISTS (SELECT 1 FROM fact f WHERE f.capture_id = c.id)",
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

/// Follow the database's changes — models, metric samples, assets, and
/// new events for the pump (a commit that logged one wakes it, whoever
/// made it) — for the life of the process.
pub fn spawn(
    db: Database,
    watermarks: Arc<ModelWatermarks>,
    events: EventBus,
    assets: crate::assets::Assets,
    pump: Arc<crate::event_pump::EventPump>,
) {
    let mut rx = db.subscribe_changes();
    tokio::spawn(async move {
        let mut lineage = Lineage::load(&db).await.unwrap_or_else(|error| {
            tracing::warn!(%error, "model lineage didn't load; no model will be reported changed");
            Lineage::default()
        });
        if let Err(error) = assets.sync_models().await {
            tracing::warn!(%error, "the materialized models didn't register");
        }
        let mut captures = CaptureListener::at_end(&db).await.unwrap_or_else(|error| {
            tracing::warn!(%error, "the capture listener starts from the beginning");
            CaptureListener::default()
        });
        loop {
            let (models, samples) = match rx.recv().await {
                Ok(tables) => {
                    if tables.contains("event_log") {
                        pump.wake();
                    }
                    if tables.contains("model_input") || tables.contains("model") {
                        if let Ok(fresh) = Lineage::load(&db).await {
                            lineage = fresh;
                        }
                        if let Err(error) = assets.sync_models().await {
                            tracing::warn!(%error, "the materialized models didn't resync");
                        }
                    }
                    assets.changed(&tables);
                    (
                        lineage.affected(tables.iter()),
                        tables.contains("metric_capture") || tables.contains("fact"),
                    )
                }
                // Missed some: anything may have changed.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    pump.wake();
                    assets.all_changed();
                    (lineage.all(), true)
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            };
            if !models.is_empty() {
                watermarks.mark(&models, Timestamp::now());
                events.emit(OxplowEvent::ModelsChanged { models });
            }
            if samples {
                match captures.landed(&db).await {
                    Ok(landed) => {
                        for (stream_id, measures) in landed {
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

    /// P4.6 (tsk491): a task edit changes the models that read `task` and
    /// the ones that read those — no others.
    #[tokio::test]
    async fn a_table_change_reaches_its_models_and_their_readers_only() {
        use oxplow_domain::stores::TaskStore as _;
        let f = crate::test_fixtures::services_with_effort().await;
        let lineage = Lineage::load(&f.svc.db).await.unwrap();
        let changed = lineage.affected(&["task".to_string()]);
        assert!(changed.contains(&"v_task".to_string()), "{changed:?}");
        assert!(!changed.contains(&"v_snapshot".to_string()), "{changed:?}");
        // Transitive: a model reading `v_test_run` changes with its inputs.
        let through = lineage.affected(&["metric_capture".to_string()]);
        assert!(through.contains(&"v_test_run".to_string()));
        assert!(
            through.contains(&"v_claim".to_string()),
            "v_claim reads v_test_run"
        );

        // End to end: a task edit is announced for v_task, with a watermark.
        let watermarks = Arc::new(ModelWatermarks::default());
        let mut rx = f.svc.events.subscribe_ui();
        spawn(
            f.svc.db.clone(),
            watermarks.clone(),
            f.svc.events.clone(),
            crate::assets::Assets::new(f.svc.db.clone(), crate::assets::COALESCE),
            f.svc.event_pump.clone(),
        );
        tokio::task::yield_now().await;
        let mut task = f.svc.task_store.get(f.task).await.unwrap().unwrap();
        task.title = "renamed".into();
        f.svc.task_store.update(&task).await.unwrap();
        let models = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Ok(OxplowEvent::ModelsChanged { models }) = rx.recv().await {
                    if models.contains(&"v_task".to_string()) {
                        return models;
                    }
                }
            }
        })
        .await
        .expect("ModelsChanged for v_task");
        assert!(!models.contains(&"v_snapshot".to_string()), "{models:?}");
        assert_eq!(watermarks.freshness(&["v_task".to_string()]).len(), 1);
        // A query's result carries its models' watermarks.
        let out = crate::sql_gateway::SqlGateway::new(f.svc.db.clone())
            .with_watermarks(watermarks.clone())
            .query_sql("SELECT count(*) FROM v_task", vec![], None)
            .await
            .unwrap();
        assert_eq!(
            out.freshness
                .iter()
                .map(|m| m.model.as_str())
                .collect::<Vec<_>>(),
            vec!["v_task"]
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
            Arc::new(ModelWatermarks::default()),
            f.svc.events.clone(),
            assets.clone(),
            f.svc.event_pump.clone(),
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

    /// P7.B2: an on-change model recomputes once after a burst of writes
    /// to its inputs, and the change loop announces it — and a model
    /// reading it — when its table is refilled, not when its inputs move.
    #[tokio::test]
    async fn a_materialized_model_recomputes_once_per_burst_and_announces_its_readers() {
        use oxplow_db::models::{ColumnDecl, ExtensionModels, Materialize, ModelDecl, ModelSource};
        use oxplow_domain::stores::TaskStore as _;
        let f = crate::test_fixtures::services_with_effort().await;
        let model =
            |name: &str, sql: &str, columns: &[(&str, &str)], on_change: bool| ModelSource {
                decl: ModelDecl {
                    name: name.into(),
                    version: 1,
                    description: format!("{name}."),
                    columns: columns
                        .iter()
                        .map(|(n, t)| ColumnDecl {
                            name: (*n).into(),
                            sql_type: (*t).into(),
                            doc: "d".into(),
                        })
                        .collect(),
                    tests: Vec::new(),
                    deprecated: Vec::new(),
                    materialize: on_change.then_some(Materialize::OnChange),
                },
                file: format!("models/{name}.sql"),
                sql: sql.into(),
                twin: None,
            };
        let errors = f
            .svc
            .db
            .compile_extension_models(vec![ExtensionModels {
                extension: "acme".into(),
                sources: vec![
                    model(
                        "titles",
                        "SELECT id, title FROM ref('task')",
                        &[("id", "INTEGER"), ("title", "TEXT")],
                        true,
                    ),
                    model(
                        "title_count",
                        "SELECT count(*) AS n FROM ref('titles')",
                        &[("n", "")],
                        false,
                    ),
                ],
            }])
            .await
            .unwrap();
        assert_eq!(errors["acme"], Vec::<String>::new());

        let assets =
            crate::assets::Assets::new(f.svc.db.clone(), std::time::Duration::from_millis(50));
        let mut rx = f.svc.events.subscribe_ui();
        spawn(
            f.svc.db.clone(),
            Arc::new(ModelWatermarks::default()),
            f.svc.events.clone(),
            assets,
            f.svc.event_pump.clone(),
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
                if let OxplowEvent::ModelsChanged { models } = e {
                    if models.contains(&"v_acme_titles".to_string()) {
                        n += 1;
                        readers |= models.contains(&"v_acme_title_count".to_string());
                    }
                }
            }
            (n, readers)
        };
        // Its first build fills it.
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(titles().await, vec!["t".to_string()]);
        let _ = announced(&mut rx);

        let mut task = f.svc.task_store.get(f.task).await.unwrap().unwrap();
        for i in 0..5 {
            task.title = format!("renamed {i}");
            f.svc.task_store.update(&task).await.unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        assert_eq!(titles().await, vec!["renamed 4".to_string()]);
        assert_eq!(
            announced(&mut rx),
            (1, true),
            "one refill, its reader with it"
        );
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
                    && !path.ends_with("models_changed.rs")
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
}
