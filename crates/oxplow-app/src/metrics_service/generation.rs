//! Prepared metric declarations joined to extension generation publication.

use super::*;
use crate::vocabulary_reactor::VocabularyErrors;
use oxplow_db::{fact_store, TxError};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) struct PreparedMetricCatalog {
    frozen: MetricsService,
    core_only: bool,
    measures: Vec<NewMeasure>,
    dimensions: Vec<NewDimension>,
    specs: BTreeMap<String, Option<NewMetricSpec>>,
    project_measures: BTreeSet<String>,
    project_specs: BTreeSet<String>,
    extension_specs: BTreeSet<String>,
    extension_dimensions: BTreeSet<String>,
}

impl MetricsService {
    pub(crate) fn prepare_generation(
        &self,
        snapshot: &Arc<crate::extension_catalog::CatalogSnapshot>,
        config: &OxplowConfig,
        core_only: bool,
    ) -> Result<PreparedMetricCatalog, VocabularyErrors> {
        let mut frozen = self.clone();
        let mut config = config.clone();
        if core_only {
            config.measures.clear();
            config.metrics.clear();
            config.dimensions.clear();
            config.collectors.clear();
        }
        frozen.config = Arc::new(RwLock::new(config.clone()));
        frozen.published_inputs = Arc::default();
        frozen.global_catalog = Arc::new(std::sync::Mutex::new(None));
        if core_only {
            frozen.global_catalog = Arc::new(std::sync::Mutex::new(Some(GlobalCatalog::default())));
        }
        frozen.extensions_cache = Arc::new(
            crate::extension_catalog::ExtensionCatalog::for_project(&self.project_dir),
        );
        frozen.extensions_cache.publish_active(snapshot.clone());
        let ext = frozen.extension_catalog();
        let (global_measures, global_dims) = frozen
            .with_global_catalog(|global| (global.measures.clone(), global.dimensions.clone()));
        let measures: Vec<_> = resolve_measures(&global_measures, &ext.measures, &config.measures)
            .into_iter()
            .map(|measure| NewMeasure {
                key: measure.key,
                title: measure.title,
                unit: measure.unit,
                subject_kind: measure.subject_kind,
                temporal_semantics: measure.temporal_semantics,
                capture_scope: measure.capture_scope,
                scope: measure.scope,
                description: measure.description,
            })
            .collect();
        let dimensions: Vec<_> =
            resolve_dimensions(&global_dims, &ext.dimensions, &config.dimensions)
                .into_iter()
                .map(|dim| NewDimension {
                    key: dim.key,
                    label: dim.label,
                    value_type: dim.value_type,
                    subject_kind: dim.subject_kind,
                    vocabulary_json: (!dim.vocabulary.is_empty()).then(|| {
                        serde_json::to_string(&dim.vocabulary).expect("dimension serializes")
                    }),
                    scope: dim.scope,
                    promoted: dim.promote,
                    entity_json: dim
                        .entity
                        .as_ref()
                        .map(|entity| serde_json::to_string(entity).expect("entity serializes")),
                })
                .collect();
        let config_state = |key: &str| {
            config
                .metrics
                .iter()
                .find(|entry| {
                    entry.use_key.as_deref() == Some(key) || entry.key.as_deref() == Some(key)
                })
                .map(|entry| entry.enabled.unwrap_or(true))
        };
        let gauges_on: BTreeSet<_> = frozen
            .fact_collectors()
            .into_iter()
            .map(|collector| collector.key)
            .collect();
        let mut specs = BTreeMap::new();
        for spec in builtin_metric_specs()
            .into_iter()
            .chain(builtin_ast_specs())
        {
            let enabled = config_state(&spec.key) != Some(false)
                && (!builtin_metrics()
                    .iter()
                    .any(|metric| metric.key == spec.key)
                    || gauges_on.contains(&spec.key));
            specs.insert(spec.key.clone(), enabled.then_some(spec));
        }
        let resolved = frozen.resolved_specs();
        let project_specs = resolved
            .iter()
            .filter(|spec| spec.scope == "project")
            .map(|spec| spec.key.clone())
            .collect();
        let extension_specs = resolved
            .iter()
            .filter(|spec| oxplow_config::scope_extension(&spec.scope).is_some())
            .map(|spec| spec.key.clone())
            .collect();
        let project_measures = measures
            .iter()
            .filter(|measure| measure.scope == "project")
            .map(|measure| measure.key.clone())
            .chain(
                resolved
                    .iter()
                    .filter(|spec| {
                        spec.scope == "project"
                            && spec
                                .entity
                                .as_ref()
                                .is_some_and(|entity| entity.time.is_none())
                    })
                    .map(|spec| spec.key.clone()),
            )
            .collect();
        let extension_dimensions = dimensions
            .iter()
            .filter(|dim| oxplow_config::scope_extension(&dim.scope).is_some())
            .map(|dim| dim.key.clone())
            .collect();
        for spec in resolved {
            specs.insert(
                spec.key.clone(),
                spec.enabled.then(|| spec_to_new_spec(&spec)),
            );
        }
        if measures.len() > 4096 || dimensions.len() > 4096 || specs.len() > 4096 {
            return Err(problem(
                "<host>",
                "metric candidate exceeds declaration limits",
            ));
        }
        let mut size = CandidateSize {
            remaining: 16 * 1024 * 1024,
        };
        serde_json::to_writer(&mut size, &(&measures, &dimensions, &specs))
            .map_err(|_| problem("<host>", "metric candidate exceeds metadata limits"))?;
        Ok(PreparedMetricCatalog {
            frozen,
            core_only,
            measures,
            dimensions,
            specs,
            project_measures,
            project_specs,
            extension_specs,
            extension_dimensions,
        })
    }
}

struct CandidateSize {
    remaining: usize,
}

impl std::io::Write for CandidateSize {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.remaining = self
            .remaining
            .checked_sub(bytes.len())
            .ok_or_else(|| std::io::Error::other("metric metadata limit exceeded"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl PreparedMetricCatalog {
    /// Called after candidate views exist, before the surrounding SQL commit.
    /// No script, network request, asynchronous wait or capture runs here.
    pub(crate) fn replace_tx(
        &self,
        tx: &rusqlite::Transaction<'_>,
    ) -> Result<(), TxError<VocabularyErrors>> {
        for measure in &self.measures {
            fact_store::upsert_measure_tx(tx, measure)?;
        }
        prune(
            tx,
            "dimension",
            "extension IS NOT NULL AND promoted = 0",
            &self.extension_dimensions,
        )?;
        let mut entity_dims = Vec::new();
        for dim in &self.dimensions {
            if let Some(json) = &dim.entity_json {
                let entity: oxplow_config::EntityDimensionSpec = serde_json::from_str(json)
                    .map_err(|_| rejected(&dim.scope, "invalid entity dimension declaration"))?;
                let probe = oxplow_config::EntitySpec {
                    view: entity.view.clone(),
                    where_: None,
                    time: None,
                    value: None,
                    aggregation: "count".into(),
                };
                check_entity(tx, &probe, Some(&entity), &dim.scope)?;
                entity_dims.push((entity.view, dim.key.clone()));
            }
            fact_store::upsert_dimension_tx(tx, dim)?;
        }
        for (key, candidate) in &self.specs {
            let Some(mut spec) = candidate.clone() else {
                tx.execute("DELETE FROM metric_spec WHERE key = ?1", [key])
                    .map_err(oxplow_db::map_sql_err)?;
                continue;
            };
            if spec.source_measure.is_some()
                && crate::metric_engine::Aggregation::parse(&spec.aggregation).is_none()
            {
                if oxplow_config::scope_extension(&spec.scope).is_some() {
                    return Err(rejected(&spec.scope, "metric aggregation is not supported"));
                }
                tx.execute("DELETE FROM metric_spec WHERE key = ?1", [key])
                    .map_err(oxplow_db::map_sql_err)?;
                continue;
            }
            if let Some(json) = &spec.entity_json {
                let entity: oxplow_config::EntitySpec = serde_json::from_str(json)
                    .map_err(|_| rejected(&spec.scope, "invalid entity metric declaration"))?;
                check_entity(tx, &entity, None, &spec.scope)?;
                let mut dims: Vec<String> = spec
                    .sliceable_dims_json
                    .as_deref()
                    .map(serde_json::from_str)
                    .transpose()
                    .map_err(|_| rejected(&spec.scope, "invalid metric dimensions"))?
                    .unwrap_or_default();
                for (view, key) in &entity_dims {
                    if *view == entity.view && !dims.contains(key) {
                        dims.push(key.clone());
                    }
                }
                spec.sliceable_dims_json = (!dims.is_empty())
                    .then(|| serde_json::to_string(&dims).expect("dimensions serialize"));
                if entity.time.is_none() {
                    fact_store::upsert_measure_tx(
                        tx,
                        &NewMeasure {
                            key: spec.key.clone(),
                            title: spec.title.clone(),
                            unit: spec.unit.clone(),
                            subject_kind: None,
                            temporal_semantics: "semi-additive".into(),
                            capture_scope: "complete".into(),
                            scope: spec.scope.clone(),
                            description: spec.description.clone(),
                        },
                    )?;
                }
            }
            fact_store::upsert_spec_tx(tx, &spec)?;
        }
        prune(tx, "metric_spec", "scope = 'project'", &self.project_specs)?;
        prune(
            tx,
            "metric_spec",
            "extension IS NOT NULL",
            &self.extension_specs,
        )?;
        // The repair baseline temporarily hides optional declarations. That
        // must never retire their measures and cascade away recorded facts.
        if !self.core_only {
            prune(tx, "measure", "scope = 'project'", &self.project_measures)?;
        }
        let specs = {
            let mut statement = tx
                .prepare("SELECT key FROM metric_spec ORDER BY key")
                .map_err(oxplow_db::map_sql_err)?;
            let keys = statement
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(oxplow_db::map_sql_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(oxplow_db::map_sql_err)?;
            keys.into_iter()
                .map(|key| {
                    fact_store::get_spec_tx(tx, &key)
                        .map_err(oxplow_db::map_sql_err)
                        .and_then(|spec| {
                            spec.ok_or_else(|| {
                                DomainError::Invalid(
                                    "metric catalog changed during publication".into(),
                                )
                            })
                        })
                })
                .collect::<Result<Vec<_>, _>>()?
        };
        let entries = self.frozen.catalog_with_specs(specs);
        tx.execute("DELETE FROM metric_catalog", [])
            .map_err(oxplow_db::map_sql_err)?;
        for entry in entries {
            tx.execute("INSERT INTO metric_catalog (key, title, kind, language, scope, enabled, target, trigger, toggleable, category)
                        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                rusqlite::params![entry.key, entry.title, entry.kind, entry.language, entry.scope, entry.enabled,
                    entry.target, entry.trigger, entry.toggleable, entry.category]).map_err(oxplow_db::map_sql_err)?;
        }
        Ok(())
    }

    pub(crate) fn publish(&self, metrics: &MetricsService) {
        let config = crate::config_service::read_config(&self.frozen.config);
        let global = self.frozen.with_global_catalog(Clone::clone);
        *metrics.published_inputs.lock() = Some(Arc::new(PublishedMetricInputs { config, global }));
    }
}

fn problem(owner: &str, message: &str) -> VocabularyErrors {
    VocabularyErrors::from([(owner.into(), vec![message.into()])])
}

fn rejected(scope: &str, message: &str) -> TxError<VocabularyErrors> {
    TxError::Aborted(problem(
        &oxplow_config::scope_extension(scope).unwrap_or_else(|| "<host>".into()),
        message,
    ))
}

fn check_entity(
    tx: &rusqlite::Transaction<'_>,
    entity: &oxplow_config::EntitySpec,
    dim: Option<&oxplow_config::EntityDimensionSpec>,
    scope: &str,
) -> Result<(), TxError<VocabularyErrors>> {
    let (sql, _) = crate::entity_metrics::build_sql(&crate::entity_metrics::EntityRead {
        spec: entity,
        dim,
        bucket: entity
            .time
            .as_ref()
            .map(|_| crate::metric_bucket::TimeBucket::Day),
        window: None,
    });
    oxplow_db::semantic_layer::check_query_on(tx, &oxplow_db::SqlQuery::new(sql)).map_err(
        |_| {
            rejected(
                scope,
                "entity metric or dimension does not compile against the candidate models",
            )
        },
    )?;
    Ok(())
}

fn prune(
    tx: &rusqlite::Transaction<'_>,
    table: &str,
    predicate: &str,
    keep: &BTreeSet<String>,
) -> Result<(), DomainError> {
    // Identifiers/predicates are fixed internal callers; keys are bound data.
    let keep = serde_json::to_string(keep)
        .map_err(|_| DomainError::Invalid("metric keys could not serialize".into()))?;
    tx.execute(&format!("DELETE FROM {table} WHERE {predicate} AND key NOT IN (SELECT value FROM json_each(?1))"), [keep])
        .map_err(oxplow_db::map_sql_err)?;
    Ok(())
}
