//! Asset subscriptions (P4.6, `.context/semantic-layer.md`
//! "Subscriptions"): which models a write changed. The database says which
//! tables each commit touched (`Database::subscribe_changes`); this follows
//! the lineage the model compiler recorded (`model_input`: a model reads a
//! table through `source()` or another model through `ref()`) from those
//! tables to every model that reads them, directly or through other models,
//! stamps each one's watermark, and announces `OxplowEvent::ModelsChanged`.
//! A lens re-runs when a model it read is in it; a query result carries its
//! models' watermarks as `freshness`.
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
                let mut st = tx
                    .prepare("SELECT input, view FROM model_input")
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

/// Follow the database's changes to models, for the life of the process.
pub fn spawn(db: Database, watermarks: Arc<ModelWatermarks>, events: EventBus) {
    let mut rx = db.subscribe_changes();
    tokio::spawn(async move {
        let mut lineage = Lineage::load(&db).await.unwrap_or_else(|error| {
            tracing::warn!(%error, "model lineage didn't load; no model will be reported changed");
            Lineage::default()
        });
        loop {
            let models = match rx.recv().await {
                Ok(tables) => {
                    if tables.contains("model_input") {
                        if let Ok(fresh) = Lineage::load(&db).await {
                            lineage = fresh;
                        }
                    }
                    lineage.affected(tables.iter())
                }
                // Missed some: anything may have changed.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => lineage.all(),
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            };
            if models.is_empty() {
                continue;
            }
            watermarks.mark(&models, Timestamp::now());
            events.emit(OxplowEvent::ModelsChanged { models });
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
        let mut rx = f.svc.events.subscribe();
        spawn(f.svc.db.clone(), watermarks.clone(), f.svc.events.clone());
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
}
