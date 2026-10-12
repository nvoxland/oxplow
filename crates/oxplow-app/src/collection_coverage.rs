//! How current the collected data behind a query is
//! (`.context/semantic-layer.md` "Completeness and coverage"). A query
//! finishing says nothing about whether its sources were collected: this
//! says, for each model a query read that a collector feeds, which
//! collector and how its last run went — observed, never inferred, and
//! `never` when nothing has run. Models core keeps current itself carry no
//! entry.
//!
//! [`Feeds`] maps each extension entity's view to the collectors that
//! declare that entity; it's restated on every catalog change. The status
//! is read at query time from `collector_run` and `contribution_health`.

use std::collections::BTreeMap;
use std::sync::RwLock;

use oxplow_db::semantic_layer::SourceCoverage;
use oxplow_db::Database;
use oxplow_domain::DomainError;
use rusqlite::OptionalExtension;

/// A collector that feeds a model.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Feeder {
    owner: String,
    id: String,
}

/// Which collectors feed which models.
#[derive(Default)]
pub struct Feeds {
    by_model: RwLock<BTreeMap<String, Vec<Feeder>>>,
}

impl Feeds {
    /// Restate from the enabled extensions' collectors.
    pub fn restate(&self, extensions: &[crate::extensions::Extension]) {
        let mut by_model: BTreeMap<String, Vec<Feeder>> = BTreeMap::new();
        for ext in extensions.iter().filter(|e| e.enabled) {
            for c in &ext.collectors {
                for entity in &c.entities {
                    by_model
                        .entry(oxplow_config::collectors::entity_view_name(
                            &ext.name,
                            &entity.name,
                        ))
                        .or_default()
                        .push(Feeder {
                            owner: ext.name.clone(),
                            id: c.id.clone(),
                        });
                }
            }
        }
        *self.by_model.write().unwrap_or_else(|e| e.into_inner()) = by_model;
    }

    fn feeders(&self, models: &[String]) -> Vec<(String, Feeder)> {
        let by_model = self.by_model.read().unwrap_or_else(|e| e.into_inner());
        models
            .iter()
            .flat_map(|m| {
                by_model
                    .get(m)
                    .into_iter()
                    .flatten()
                    .map(move |f| (m.clone(), f.clone()))
            })
            .collect()
    }

    /// The coverage of `models`: one entry per collector feeding each.
    pub async fn coverage(
        &self,
        db: &Database,
        models: &[String],
    ) -> Result<Vec<SourceCoverage>, DomainError> {
        let feeders = self.feeders(models);
        if feeders.is_empty() {
            return Ok(Vec::new());
        }
        let now = oxplow_domain::Timestamp::now().to_string();
        db.read(move |c| {
            feeders
                .into_iter()
                .map(|(model, f)| status_of(c, &now, model, &f))
                .collect()
        })
        .await
    }
}

fn status_of(
    c: &rusqlite::Connection,
    now: &str,
    model: String,
    f: &Feeder,
) -> Result<SourceCoverage, DomainError> {
    let sql = |e: rusqlite::Error| DomainError::Storage(e.to_string());
    let run: Option<(String, String, Option<String>)> = c
        .query_row(
            "SELECT status, last_run_at, error FROM collector_run WHERE owner = ?1 AND id = ?2",
            [&f.owner, &f.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(sql)?;
    let health: Option<(String, Option<String>, Option<String>)> = c
        .query_row(
            "SELECT state, reason, next_due_at FROM contribution_health
              WHERE extension = ?1 AND contribution = ?2 AND kind = 'collector'",
            [&f.owner, &f.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(sql)?;
    let (status, at, error) = match (&run, &health) {
        (_, Some((state, reason, _))) if state == "disabled" => (
            "disabled",
            run.as_ref().map(|r| r.1.clone()),
            reason.clone(),
        ),
        (None, _) => ("never", None, None),
        (Some((status, at, error)), _) if status == "error" => {
            ("failed", Some(at.clone()), error.clone())
        }
        (Some((status, at, _)), _) if status == "needs_approval" => {
            ("needs_approval", Some(at.clone()), None)
        }
        (Some((_, at, _)), Some((_, _, Some(due)))) if due.as_str() < now => {
            ("stale", Some(at.clone()), None)
        }
        (Some((_, at, _)), _) => ("current", Some(at.clone()), None),
    };
    Ok(SourceCoverage {
        model,
        source: format!("collector:{}/{}", f.owner, f.id),
        status: status.into(),
        at,
        error,
    })
}

#[cfg(test)]
mod tests {
    use oxplow_db::CollectorRun;

    async fn fixture() -> crate::test_fixtures::EffortFixture {
        let fx = crate::test_fixtures::services_with_effort().await;
        let dir = fx.svc.layout.project_dir.join("oxplow/extensions/my-gh");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("extension.yaml"),
            "manifest: 2\nname: my-gh\nintent:\n  purpose: test\ncollectors:\n  - id: gh\n    runtime: exec\n    entry: sync.sh\n    entities:\n      - { name: pr, key: number, columns: { number: int, title: text } }\n",
        )
        .unwrap();
        std::fs::write(dir.join("sync.sh"), "#!/bin/sh\necho '{}'\n").unwrap();
        fx.svc.extension_catalog.changed();
        crate::test_fixtures::activate_with_problems(&fx.svc).await;
        fx
    }

    fn run(status: &str, error: Option<&str>) -> CollectorRun {
        CollectorRun {
            owner: "my-gh".into(),
            id: "gh".into(),
            status: status.into(),
            last_run_at: "2026-10-09T12:00:00.000000Z".into(),
            error: error.map(str::to_string),
            row_counts: Default::default(),
            cursor: None,
            last_event_id: None,
            run_id: None,
        }
    }

    /// A model a collector feeds says how the collector stands: never run,
    /// then its last run's failure, then current.
    #[tokio::test]
    async fn a_fed_model_says_how_its_collection_stands() {
        let fx = fixture().await;
        let models = vec!["v_my_gh_pr".to_string(), "v_effort".to_string()];
        let coverage = fx.svc.feeds.coverage(&fx.svc.db, &models).await.unwrap();
        assert_eq!(coverage.len(), 1, "core's models carry none: {coverage:?}");
        assert_eq!(
            (coverage[0].source.as_str(), coverage[0].status.as_str()),
            ("collector:my-gh/gh", "never")
        );
        let store = oxplow_db::SqliteCollectorStore::new(fx.svc.db.clone());
        store
            .record_run(run("error", Some("rate limited")))
            .await
            .unwrap();
        let failed = fx.svc.feeds.coverage(&fx.svc.db, &models).await.unwrap();
        assert_eq!(failed[0].status, "failed");
        assert_eq!(failed[0].error.as_deref(), Some("rate limited"));
        store.record_run(run("ok", None)).await.unwrap();
        let current = fx.svc.feeds.coverage(&fx.svc.db, &models).await.unwrap();
        assert_eq!(current[0].status, "current");
        assert!(current[0].at.is_some());
    }

    /// Every result through the gateway says which catalog generation it
    /// ran against; a change to the extensions changes it.
    #[tokio::test]
    async fn a_result_names_the_catalog_generation() {
        let fx = fixture().await;
        let query = || oxplow_db::SqlQuery::new("SELECT count(*) FROM v_effort");
        let first = fx.svc.sql.run(query()).await.unwrap();
        assert!(first.coverage.is_empty());
        let generation = first.generation.clone().expect("a generation");
        assert_eq!(
            fx.svc.sql.run(query()).await.unwrap().generation,
            Some(generation.clone())
        );
        std::fs::write(
            fx.svc
                .layout
                .project_dir
                .join("oxplow/extensions/my-gh/extra.txt"),
            "x",
        )
        .unwrap();
        crate::test_fixtures::activate(&fx.svc).await;
        assert_ne!(
            fx.svc.sql.run(query()).await.unwrap().generation,
            Some(generation)
        );
    }
}
