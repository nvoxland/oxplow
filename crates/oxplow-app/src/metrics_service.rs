//! The metric runner (epic tsk213, P3): ties config-declared `metrics:` entries
//! to the substrate. It seeds a `metric_definition` per resolved metric and runs
//! each on its trigger — `on-snapshot` after a snapshot is captured,
//! `on-effort-complete` when an effort closes, `manual` via MCP.
//!
//! A gauge collector (Starlark/jaq/exec, built from the entry's `compute:`) is
//! run with a [`GaugeHost`] exposing the captured snapshot's file map, so a
//! tree-derived gauge can call `files(glob)` / `ast_query(...)`. Each
//! `MetricReport.sample` becomes a durable `metric_sample`.
//!
//! Best-effort, like the other producers (`token_usage.rs` / `collection.rs`):
//! a compute/write error is logged via `tracing::warn!`, never propagated, and
//! never blocks the host path. Successful runs emit
//! `OxplowEvent::MetricSamplesChanged { stream_id }`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use oxplow_collect_plugin::{builtin_metrics, Collector, CollectorInput, CollectorKind, GaugeHost};
use oxplow_config::{
    global_config_dir, load_global_dimension_entries, load_global_measure_entries,
    load_global_metric_entries, resolve_dimensions, resolve_measures, resolve_metrics,
    DimensionEntry, MeasureEntry, MetricComputeConfig, MetricEntry, OxplowConfig, ResolvedMetric,
};
use oxplow_db::{
    NewDimension, NewMeasure, NewMetricDefinition, NewMetricRun, NewMetricSample, NewMetricSpec,
    SnapshotStorage, SqliteFactStore, SqliteMetricStore, SqliteSnapshotStore,
    SqliteTaskEffortStore, SqliteThreadStore, TaskEffortStore,
};
use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{DomainError, EffortId, StreamId, ThreadId};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::blob_store::BlobStore;
use crate::events::{EventBus, OxplowEvent, SnapshotSourceKind};
use crate::producer_metrics::builtin_producer_metrics;
use crate::snapshot_content::read_snapshot_content;

const DEFAULT_MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

/// Runs config-declared metrics into the substrate. Cheap to clone (a handle of
/// leaf `Arc`s) — deliberately NOT holding `Arc<Services>`, to avoid a cycle.
#[derive(Clone)]
pub struct MetricsService {
    metrics: Arc<SqliteMetricStore>,
    snapshot_store: Arc<SqliteSnapshotStore>,
    thread_store: Arc<SqliteThreadStore>,
    effort_store: Arc<SqliteTaskEffortStore>,
    blobs: BlobStore,
    config: Arc<RwLock<OxplowConfig>>,
    project_dir: PathBuf,
    events: EventBus,
    /// Override for the global config dir (the parent of `metrics/`). `None` →
    /// the platform `global_config_dir()`. A field (not the free fn) so tests
    /// can point it at a tempdir without racing on a process-global env var.
    global_dir: Option<PathBuf>,
    /// The fact substrate, for seeding config-declared `measures:`/`dimensions:`
    /// into the catalog (epic tsk12, E). `None` in test fixtures that don't
    /// exercise catalog seeding; wired at boot via [`Self::with_fact_store`].
    fact_store: Option<Arc<SqliteFactStore>>,
}

/// One row in the **available** metric catalog (built-in ∪ global ∪ project) for
/// the Catalog UI (tsk219, P4): what the metric is + whether the project has it
/// enabled. Distinct from `MetricDefinition` (the seeded substrate row) — a
/// built-in appears here even before it's enabled/seeded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct MetricCatalogEntry {
    pub key: String,
    pub title: String,
    pub kind: String,
    pub language: Option<String>,
    /// `built-in` | `global` | `project`.
    pub scope: String,
    /// Active in this project's `.oxplow/project.yaml` `metrics:` block. Always `true`
    /// for non-toggleable (always-on) producer/plugin metrics.
    pub enabled: bool,
    pub target: Option<f64>,
    pub trigger: String,
    /// Whether this metric can be enabled/disabled + overridden from config.
    /// `true` for the bundled code gauges (`use:`-able) and project/global
    /// `metrics:` entries; `false` for always-on producers (tokens, tests,
    /// coverage, analysis, lifecycle, nudges) and plugin-seeded definitions —
    /// those are free side-bands, not opt-in compute. The real axis is
    /// always-on vs toggleable; "built-in vs hardcoded" was an artifact (tsk284).
    pub toggleable: bool,
    /// `operational` | `testing` | `static-quality` | `custom` — drives the
    /// Catalog page's grouping.
    pub category: Option<String>,
}

/// The per-trigger context every gauge run is stamped with.
struct GaugeRunContext {
    stream_val: i64,
    thread_id: Option<i64>,
    trigger: &'static str,
    snapshot_id: Option<i64>,
    closest_git_version: Option<String>,
    git_version_exact: bool,
    branch: Option<String>,
    /// Default `(subject_kind, subject_ref)` when a sample omits its subject.
    subject_default: Option<(String, String)>,
}

impl MetricsService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        metrics: Arc<SqliteMetricStore>,
        snapshot_store: Arc<SqliteSnapshotStore>,
        thread_store: Arc<SqliteThreadStore>,
        effort_store: Arc<SqliteTaskEffortStore>,
        blobs: BlobStore,
        config: Arc<RwLock<OxplowConfig>>,
        project_dir: PathBuf,
        events: EventBus,
    ) -> Self {
        Self {
            metrics,
            snapshot_store,
            thread_store,
            effort_store,
            blobs,
            config,
            project_dir,
            events,
            global_dir: None,
            fact_store: None,
        }
    }

    /// Override the global config dir (test seam; default `global_config_dir()`).
    pub fn with_global_dir(mut self, dir: PathBuf) -> Self {
        self.global_dir = Some(dir);
        self
    }

    /// Wire the fact substrate so `run()` seeds config-declared measures +
    /// dimensions into the catalog beside the migration-seeded built-ins.
    pub fn with_fact_store(mut self, fact_store: Arc<SqliteFactStore>) -> Self {
        self.fact_store = Some(fact_store);
        self
    }

    /// The effective global config dir (the field override, else the platform
    /// `global_config_dir()`); `metrics/` hangs under it.
    fn effective_global_dir(&self) -> Option<PathBuf> {
        self.global_dir.clone().or_else(global_config_dir)
    }

    /// Base dir a metric's `compute.entryFile` / `report` resolves against:
    /// `<global>/metrics` for a global-scope metric, else the project dir
    /// (tsk235). Falls back to the project dir if no global dir is available.
    fn script_base_dir(&self, metric: &ResolvedMetric) -> PathBuf {
        if metric.scope == "global" {
            if let Some(g) = self.effective_global_dir() {
                return g.join("metrics");
            }
        }
        self.project_dir.clone()
    }

    /// The active, resolved metrics for this project (built-in ∪ global ∪
    /// project, precedence project > global > built-in). Built-ins are the
    /// bundled catalog (`oxplow_collect_plugin::builtin_metrics`, tsk218); a
    /// project activates one with `metrics: - use: oxplow.<lang>.<name>`.
    fn resolved_metrics(&self) -> Vec<ResolvedMetric> {
        let project = self
            .config
            .read()
            .map(|c| c.metrics.clone())
            .unwrap_or_default();
        let global = self
            .effective_global_dir()
            .map(|d| load_global_metric_entries(&d))
            .unwrap_or_default();
        let builtin = builtin_metric_entries();
        resolve_metrics(&builtin, &global, &project)
    }

    fn max_file_bytes(&self) -> u64 {
        self.config
            .read()
            .map(|c| c.snapshot_max_file_bytes)
            .unwrap_or(DEFAULT_MAX_FILE_BYTES)
    }

    /// Upsert a `metric_definition` for every resolved entry so the catalog /
    /// Metrics page list them and samples have a stable FK — even before the
    /// first run. Idempotent; best-effort. Returns the count seeded.
    pub async fn seed_definitions(&self) -> usize {
        let mut n = 0;
        for m in self.resolved_metrics() {
            match self.metrics.upsert_definition(metric_definition(&m)).await {
                Ok(_) => n += 1,
                Err(e) => {
                    tracing::warn!(key = %m.key, error = %e, "failed to seed metric definition")
                }
            }
        }
        n
    }

    /// Seed the fact-substrate catalogs (`measure` + `dimension`) from config —
    /// the pluggable-data half of the substrate (epic tsk12, E). Resolves the
    /// global + project `measures:`/`dimensions:` blocks and upserts each beside
    /// the migration-seeded `oxplow.*` built-ins. Best-effort (a write error is
    /// logged, never propagated); idempotent (upsert by key). No-op if no fact
    /// store is wired. Returns `(measures, dimensions)` seeded.
    ///
    /// The dimension `promote` flag (a generated column + index) is honored by a
    /// later `promote_dimension` step; this seeds the catalog row only.
    pub async fn seed_catalog(&self) -> (usize, usize) {
        let Some(facts) = self.fact_store.as_ref() else {
            return (0, 0);
        };
        let (project_measures, project_dims) = self
            .config
            .read()
            .map(|c| (c.measures.clone(), c.dimensions.clone()))
            .unwrap_or_default();
        let global_dir = self.effective_global_dir();
        let global_measures = global_dir
            .as_deref()
            .map(load_global_measure_entries)
            .unwrap_or_default();
        let global_dims = global_dir
            .as_deref()
            .map(load_global_dimension_entries)
            .unwrap_or_default();

        let mut m = 0;
        for rm in resolve_measures(&global_measures, &project_measures) {
            let nm = NewMeasure {
                key: rm.key.clone(),
                title: rm.title,
                unit: rm.unit,
                subject_kind: rm.subject_kind,
                temporal_semantics: rm.temporal_semantics,
                component_role: rm.component_role,
                scope: rm.scope,
                description: rm.description,
            };
            match facts.upsert_measure(nm).await {
                Ok(_) => m += 1,
                Err(e) => tracing::warn!(key = %rm.key, error = %e, "failed to seed measure"),
            }
        }
        let mut d = 0;
        for rd in resolve_dimensions(&global_dims, &project_dims) {
            let vocabulary_json = (!rd.vocabulary.is_empty())
                .then(|| serde_json::to_string(&rd.vocabulary).ok())
                .flatten();
            let nd = NewDimension {
                key: rd.key.clone(),
                label: rd.label,
                value_type: rd.value_type,
                subject_kind: rd.subject_kind,
                vocabulary_json,
                scope: rd.scope,
            };
            match facts.upsert_dimension(nd).await {
                Ok(()) => d += 1,
                Err(e) => tracing::warn!(key = %rd.key, error = %e, "failed to seed dimension"),
            }
        }
        // Built-in metric SPECS (epic tsk12): the bundled code metrics are now
        // COUNT aggregations over per-item facts, not baked sample streams. Seed
        // them beside the migration's built-in measures (idempotent). Config /
        // global spec seeding lands with the read-flip (tsk26).
        for spec in builtin_metric_specs() {
            if let Err(e) = facts.upsert_spec(spec.clone()).await {
                tracing::warn!(key = %spec.key, error = %e, "failed to seed built-in metric spec");
            }
        }
        (m, d)
    }

    /// The **available** catalog (built-in ∪ global ∪ project) with each entry's
    /// enabled-in-this-project flag — the Catalog page's read (tsk219). A
    /// built-in shows up even before it's `use:`d/seeded.
    /// The full metric registry for this project — **everything available**,
    /// not just metrics with recorded data. Four sources, deduped by key:
    /// 1. bundled code gauges (`builtin_metrics()`) — toggleable, shown even
    ///    before they're enabled;
    /// 2. project/global `metrics:` entries — toggleable;
    /// 3. built-in always-on producers (`builtin_producer_metrics()`) — tokens,
    ///    tests, coverage, analysis, lifecycle, nudges — `toggleable: false`,
    ///    listed even with zero recorded data so the user can see they exist
    ///    (tsk286);
    /// 4. every other seeded `metric_definition` — installed plugin metrics (and
    ///    legacy rows) not covered above. Also `toggleable: false`.
    pub async fn catalog(&self) -> Vec<MetricCatalogEntry> {
        let resolved = self.resolved_metrics();
        let by_key: std::collections::HashMap<&str, &_> =
            resolved.iter().map(|m| (m.key.as_str(), m)).collect();
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for b in builtin_metrics() {
            seen.insert(b.key.to_string());
            // When enabled, surface the *resolved* target/trigger (so a project
            // override shows through, tsk233); otherwise the built-in defaults.
            let r = by_key.get(b.key);
            out.push(MetricCatalogEntry {
                key: b.key.to_string(),
                title: b.title.to_string(),
                kind: b.kind.to_string(),
                language: Some(b.language.to_string()),
                scope: "built-in".to_string(),
                enabled: r.is_some(),
                target: r.map_or(b.target, |m| m.target),
                trigger: r.map_or_else(|| b.trigger.to_string(), |m| m.trigger.clone()),
                toggleable: true,
                category: Some("custom".to_string()),
            });
        }
        // Project/global-defined metrics not already shown as a built-in.
        for m in &resolved {
            if seen.insert(m.key.clone()) {
                out.push(MetricCatalogEntry {
                    key: m.key.clone(),
                    title: m.title.clone(),
                    kind: m.kind.clone(),
                    language: m.language.clone(),
                    scope: m.scope.clone(),
                    enabled: true,
                    target: m.target,
                    trigger: m.trigger.clone(),
                    toggleable: true,
                    category: Some("custom".to_string()),
                });
            }
        }
        // Built-in always-on producer metrics — listed even with zero recorded
        // data, so the registry is complete the moment a project opens (tsk286).
        for p in builtin_producer_metrics() {
            if seen.insert(p.key.to_string()) {
                out.push(MetricCatalogEntry {
                    key: p.key.to_string(),
                    title: p.title.to_string(),
                    kind: p.kind.to_string(),
                    language: None,
                    scope: "built-in".to_string(),
                    enabled: true,
                    target: None,
                    trigger: "auto".to_string(),
                    toggleable: false,
                    category: Some(p.category.to_string()),
                });
            }
        }
        // Every other seeded definition — installed plugin metrics (and legacy
        // rows) not covered above. Best-effort: a store read error just yields
        // the set assembled so far.
        if let Ok(defs) = self.metrics.list_definitions().await {
            for d in defs {
                if seen.insert(d.key.clone()) {
                    out.push(MetricCatalogEntry {
                        key: d.key.clone(),
                        title: d.title.clone(),
                        kind: d.kind.clone(),
                        language: d.language.clone(),
                        scope: d.scope.clone(),
                        enabled: true,
                        target: d.target,
                        // No config trigger for a producer-seeded metric; it runs
                        // on its producer's own cadence.
                        trigger: "auto".to_string(),
                        toggleable: false,
                        category: d.category.clone(),
                    });
                }
            }
        }
        out.sort_by(|a, b| a.key.cmp(&b.key));
        out
    }

    /// Enable (add a `use:` entry) or disable (remove all entries for the key)
    /// a metric in this project's `.oxplow/project.yaml`, then reseed. Persists the
    /// config to disk + emits `ConfigChanged` (the Catalog toggle, tsk219).
    pub async fn set_metric_enabled(&self, key: &str, enabled: bool) -> Result<(), String> {
        {
            let mut cfg = self
                .config
                .write()
                .map_err(|_| "config lock poisoned".to_string())?;
            let present = cfg
                .metrics
                .iter()
                .any(|e| e.use_key.as_deref() == Some(key) || e.key.as_deref() == Some(key));
            if enabled && !present {
                cfg.metrics.push(MetricEntry {
                    use_key: Some(key.to_string()),
                    ..Default::default()
                });
            } else if !enabled {
                cfg.metrics
                    .retain(|e| e.use_key.as_deref() != Some(key) && e.key.as_deref() != Some(key));
            }
            oxplow_config::write_project_config(&self.project_dir, &cfg)
                .map_err(|e| e.to_string())?;
        }
        self.events.emit(OxplowEvent::ConfigChanged);
        self.seed_definitions().await;
        Ok(())
    }

    /// Set the `target` / `trigger` override for a metric in this project's
    /// `.oxplow/project.yaml`, then reseed (the Catalog inline edit, tsk233). Enabling
    /// it if not already present (an override implies the metric is active);
    /// `None` for a field clears that override (falls back to the definition's
    /// default). Persists + emits `ConfigChanged`.
    pub async fn set_metric_override(&self, key: &str, target: Option<f64>) -> Result<(), String> {
        {
            let mut cfg = self
                .config
                .write()
                .map_err(|_| "config lock poisoned".to_string())?;
            let entry = cfg
                .metrics
                .iter_mut()
                .find(|e| e.use_key.as_deref() == Some(key) || e.key.as_deref() == Some(key));
            match entry {
                // Only `target` is project-overridable; `trigger` is inherent to
                // the definition and never set here (tsk290).
                Some(e) => {
                    e.target = target;
                }
                None => cfg.metrics.push(MetricEntry {
                    use_key: Some(key.to_string()),
                    target,
                    ..Default::default()
                }),
            }
            oxplow_config::write_project_config(&self.project_dir, &cfg)
                .map_err(|e| e.to_string())?;
        }
        self.events.emit(OxplowEvent::ConfigChanged);
        self.seed_definitions().await;
        Ok(())
    }

    /// Scaffold a new gauge metric (tsk234/tsk235): write a starter Starlark
    /// script + a `key:` `metrics:` entry, then reseed. Returns the path to the
    /// stub. The metric runs `on-snapshot` and charts as soon as it has samples.
    ///
    /// `scope`: `project` (default) writes the script under `oxplow/metrics/`
    /// and the entry into `.oxplow/project.yaml`, returning the **project-relative** path
    /// (so the UI can open it). `global` writes both under
    /// `<global_config_dir>/metrics/` (shared across the user's projects),
    /// returning the **absolute** script path. The runner resolves each scope's
    /// `entryFile` against the matching base dir (`script_base_dir`).
    pub async fn scaffold_metric(
        &self,
        key: &str,
        title: Option<String>,
        language: Option<String>,
        glob: Option<String>,
        scope: Option<String>,
    ) -> Result<String, String> {
        let key = key.trim();
        if key.is_empty() || !key.contains('.') {
            return Err("key must be namespaced, e.g. acme.my_metric".to_string());
        }
        if key.starts_with("oxplow.") {
            return Err("`oxplow.` is reserved for built-in metrics".to_string());
        }
        let global = matches!(scope.as_deref(), Some("global"));
        let glob = glob
            .filter(|g| !g.is_empty())
            .unwrap_or_else(|| "**/*".into());
        let language = language.filter(|l| !l.is_empty());
        let slug: String = key
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        let script = starter_metric_script(key, &glob, language.as_deref());
        let entry = MetricEntry {
            key: Some(key.to_string()),
            title: title.filter(|t| !t.is_empty()),
            kind: Some("gauge".to_string()),
            language,
            trigger: Some("on-snapshot".to_string()),
            compute: Some(MetricComputeConfig {
                runtime: "starlark".to_string(),
                input: None,
                // entryFile is relative to the scope's base dir.
                entry_file: Some(if global {
                    format!("{slug}.star")
                } else {
                    format!("oxplow/metrics/{slug}.star")
                }),
                args: vec![],
                report: None,
            }),
            ..Default::default()
        };

        let returned_path = if global {
            let gdir = self
                .effective_global_dir()
                .ok_or_else(|| "no global config dir available on this platform".to_string())?;
            let mdir = gdir.join("metrics");
            let existing = load_global_metric_entries(&gdir);
            if existing
                .iter()
                .any(|e| e.key.as_deref() == Some(key) || e.use_key.as_deref() == Some(key))
            {
                return Err(format!("global metric `{key}` already exists"));
            }
            std::fs::create_dir_all(&mdir).map_err(|e| e.to_string())?;
            let script_abs = mdir.join(format!("{slug}.star"));
            if !script_abs.exists() {
                std::fs::write(&script_abs, script).map_err(|e| e.to_string())?;
            }
            oxplow_config::write_global_metrics_file(&mdir.join(format!("{slug}.yaml")), &[entry])
                .map_err(|e| e.to_string())?;
            // A global `key:` define is library content — only active once the
            // project opts in. Enable it here with a project `use:` so it charts
            // in this project (and stays reusable across the user's others).
            {
                let mut cfg = self
                    .config
                    .write()
                    .map_err(|_| "config lock poisoned".to_string())?;
                if !cfg
                    .metrics
                    .iter()
                    .any(|e| e.use_key.as_deref() == Some(key) || e.key.as_deref() == Some(key))
                {
                    cfg.metrics.push(MetricEntry {
                        use_key: Some(key.to_string()),
                        ..Default::default()
                    });
                    oxplow_config::write_project_config(&self.project_dir, &cfg)
                        .map_err(|e| e.to_string())?;
                }
            }
            script_abs.to_string_lossy().into_owned()
        } else {
            let script_rel = format!("oxplow/metrics/{slug}.star");
            {
                let mut cfg = self
                    .config
                    .write()
                    .map_err(|_| "config lock poisoned".to_string())?;
                if cfg
                    .metrics
                    .iter()
                    .any(|e| e.use_key.as_deref() == Some(key) || e.key.as_deref() == Some(key))
                {
                    return Err(format!(
                        "metric `{key}` already exists in .oxplow/project.yaml"
                    ));
                }
                let abs = self.project_dir.join(&script_rel);
                if let Some(parent) = abs.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                // Don't clobber an existing script (a re-run after manual edits).
                if !abs.exists() {
                    std::fs::write(&abs, script).map_err(|e| e.to_string())?;
                }
                cfg.metrics.push(entry);
                oxplow_config::write_project_config(&self.project_dir, &cfg)
                    .map_err(|e| e.to_string())?;
            }
            script_rel
        };

        self.events.emit(OxplowEvent::ConfigChanged);
        self.seed_definitions().await;
        Ok(returned_path)
    }

    /// Scaffold a custom **measure** (a new fact TYPE) — epic tsk12, E. Appends a
    /// `measures:` entry to `.oxplow/project.yaml` (project scope, default) or
    /// writes a shareable `<global>/measures/<slug>.yaml` (global scope), then
    /// reseeds the catalog. Returns the created measure key. A global measure is
    /// active in every project automatically (`seed_catalog` loads global +
    /// project), so — unlike a metric — no project `use:` opt-in is written.
    pub async fn scaffold_measure(
        &self,
        key: &str,
        title: Option<String>,
        unit: Option<String>,
        subject_kind: Option<String>,
        temporal_semantics: Option<String>,
        scope: Option<String>,
    ) -> Result<String, String> {
        let key = key.trim();
        if key.is_empty() || !key.contains('.') {
            return Err("key must be namespaced, e.g. acme.api_latency".to_string());
        }
        if key.starts_with("oxplow.") {
            return Err("`oxplow.` is reserved for built-in measures".to_string());
        }
        let entry = MeasureEntry {
            key: Some(key.to_string()),
            title: title.filter(|t| !t.is_empty()),
            unit: unit.filter(|u| !u.is_empty()),
            subject_kind: subject_kind.filter(|s| !s.is_empty()),
            temporal_semantics: temporal_semantics.filter(|s| !s.is_empty()),
            component_role: None,
            description: None,
        };
        if matches!(scope.as_deref(), Some("global")) {
            let gdir = self
                .effective_global_dir()
                .ok_or_else(|| "no global config dir available on this platform".to_string())?;
            if load_global_measure_entries(&gdir)
                .iter()
                .any(|e| e.key.as_deref() == Some(key))
            {
                return Err(format!("global measure `{key}` already exists"));
            }
            let slug = slugify(key);
            oxplow_config::write_global_measures_file(
                &gdir.join("measures").join(format!("{slug}.yaml")),
                &[entry],
            )
            .map_err(|e| e.to_string())?;
        } else {
            let mut cfg = self
                .config
                .write()
                .map_err(|_| "config lock poisoned".to_string())?;
            if cfg.measures.iter().any(|e| e.key.as_deref() == Some(key)) {
                return Err(format!(
                    "measure `{key}` already exists in .oxplow/project.yaml"
                ));
            }
            cfg.measures.push(entry);
            oxplow_config::write_project_config(&self.project_dir, &cfg)
                .map_err(|e| e.to_string())?;
        }
        self.events.emit(OxplowEvent::ConfigChanged);
        self.seed_catalog().await;
        Ok(key.to_string())
    }

    /// Scaffold a custom **dimension** (a new conformed slice axis) — epic tsk12,
    /// E. Analogous to [`Self::scaffold_measure`]: appends a `dimensions:` entry
    /// (project) or writes `<global>/dimensions/<slug>.yaml` (global), reseeds,
    /// and returns the created key.
    pub async fn scaffold_dimension(
        &self,
        key: &str,
        label: Option<String>,
        value_type: Option<String>,
        scope: Option<String>,
    ) -> Result<String, String> {
        let key = key.trim();
        if key.is_empty() || !key.contains('.') {
            return Err("key must be namespaced, e.g. acme.license".to_string());
        }
        if key.starts_with("oxplow.") {
            return Err("`oxplow.` is reserved for built-in dimensions".to_string());
        }
        let entry = DimensionEntry {
            key: Some(key.to_string()),
            label: label.filter(|l| !l.is_empty()),
            value_type: value_type.filter(|v| !v.is_empty()),
            subject_kind: None,
            vocabulary: vec![],
            promote: false,
        };
        if matches!(scope.as_deref(), Some("global")) {
            let gdir = self
                .effective_global_dir()
                .ok_or_else(|| "no global config dir available on this platform".to_string())?;
            if load_global_dimension_entries(&gdir)
                .iter()
                .any(|e| e.key.as_deref() == Some(key))
            {
                return Err(format!("global dimension `{key}` already exists"));
            }
            let slug = slugify(key);
            oxplow_config::write_global_dimensions_file(
                &gdir.join("dimensions").join(format!("{slug}.yaml")),
                &[entry],
            )
            .map_err(|e| e.to_string())?;
        } else {
            let mut cfg = self
                .config
                .write()
                .map_err(|_| "config lock poisoned".to_string())?;
            if cfg.dimensions.iter().any(|e| e.key.as_deref() == Some(key)) {
                return Err(format!(
                    "dimension `{key}` already exists in .oxplow/project.yaml"
                ));
            }
            cfg.dimensions.push(entry);
            oxplow_config::write_project_config(&self.project_dir, &cfg)
                .map_err(|e| e.to_string())?;
        }
        self.events.emit(OxplowEvent::ConfigChanged);
        self.seed_catalog().await;
        Ok(key.to_string())
    }

    /// Event loop: seed once, then reseed on `ConfigChanged` and run on-snapshot
    /// gauges when a snapshot batch lands. Spawned at boot (see `boot.rs`).
    pub async fn run(self, mut rx: tokio::sync::broadcast::Receiver<OxplowEvent>) {
        self.seed_definitions().await;
        self.seed_catalog().await;
        loop {
            match rx.recv().await {
                Ok(OxplowEvent::ConfigChanged) => {
                    self.seed_definitions().await;
                    self.seed_catalog().await;
                }
                Ok(OxplowEvent::FileSnapshotsBatchCreated {
                    stream_id: Some(stream_id),
                    snapshot_id,
                    file_count,
                    source,
                    ..
                }) => {
                    // A git-refs re-stamp with no content change can't move a
                    // tree metric — skip it to avoid recompute storms.
                    if matches!(source, SnapshotSourceKind::GitRefs) && file_count == 0 {
                        continue;
                    }
                    self.run_snapshot_gauges(stream_id, snapshot_id).await;
                }
                Ok(OxplowEvent::FileSnapshotCreated {
                    stream_id: Some(stream_id),
                    snapshot_id,
                    ..
                }) => {
                    self.run_snapshot_gauges(stream_id, snapshot_id).await;
                }
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    }

    /// Run every enabled `on-snapshot` gauge against the just-captured snapshot.
    async fn run_snapshot_gauges(&self, stream_id: StreamId, snapshot_id: i64) {
        let metrics: Vec<ResolvedMetric> = self
            .resolved_metrics()
            .into_iter()
            .filter(|m| m.trigger == "on-snapshot" && m.kind == "gauge")
            .collect();
        if metrics.is_empty() {
            return;
        }
        // Build the snapshot file map once and share it across every gauge of
        // this run via an Arc (no per-gauge clone of the whole map).
        let files = Arc::new(self.build_file_map(snapshot_id).await);
        let ctx = self
            .snapshot_context(stream_id.value(), None, "on-snapshot", snapshot_id, None)
            .await;
        for m in &metrics {
            self.run_one_gauge(m, &ctx, files.clone()).await;
        }
    }

    /// Run every enabled `on-effort-complete` gauge when an effort closes. The
    /// file map comes from the effort's end snapshot (the worktree as it stood
    /// at close); samples default their subject to the effort.
    pub async fn run_effort_complete_gauges(&self, thread_id: &ThreadId, effort_id: &EffortId) {
        let metrics: Vec<ResolvedMetric> = self
            .resolved_metrics()
            .into_iter()
            .filter(|m| m.trigger == "on-effort-complete" && m.kind == "gauge")
            .collect();
        if metrics.is_empty() {
            return;
        }
        let stream_val = match self.thread_store.get(thread_id).await {
            Ok(Some(t)) => t.stream_id.value(),
            _ => return,
        };
        let snapshot_id = match self.effort_store.get_effort(effort_id).await {
            Ok(Some(e)) => e.end_snapshot_id,
            _ => None,
        };
        let files = Arc::new(match snapshot_id {
            Some(sid) => self.build_file_map(sid).await,
            None => HashMap::new(),
        });
        let subject = Some(("effort".to_string(), effort_id.to_string()));
        let ctx = self
            .snapshot_context(
                stream_val,
                Some(thread_id.value()),
                "on-effort-complete",
                snapshot_id.unwrap_or(0),
                subject,
            )
            .await;
        for m in &metrics {
            self.run_one_gauge(m, &ctx, files.clone()).await;
        }
    }

    /// Manually run one configured metric by key, against the stream's latest
    /// snapshot. Returns the number of samples recorded, or an error string.
    /// (The MCP `run_metric` tool, tsk226, calls this.)
    pub async fn run_metric_by_key(
        &self,
        key: &str,
        stream: Option<StreamId>,
    ) -> Result<usize, String> {
        let metric = self
            .resolved_metrics()
            .into_iter()
            .find(|m| m.key == key)
            .ok_or_else(|| format!("no configured metric with key \"{key}\""))?;
        let stream_val = match stream {
            Some(s) => s.value(),
            None => 1, // primary stream default
        };
        let snapshot_id = self
            .snapshot_store
            .latest_snapshot_id_for_stream(StreamId::new(stream_val))
            .await
            .ok()
            .flatten();
        let files = Arc::new(match snapshot_id {
            Some(sid) => self.build_file_map(sid).await,
            None => HashMap::new(),
        });
        let ctx = self
            .snapshot_context(stream_val, None, "manual", snapshot_id.unwrap_or(0), None)
            .await;
        Ok(self.run_one_gauge(&metric, &ctx, files).await)
    }

    /// Build the snapshot file map (repo-relative path → UTF-8 content) for
    /// `snapshot_id`, skipping deleted/oversize/binary/over-large files. The
    /// blob reads are blocking I/O, so they run on a blocking thread.
    async fn build_file_map(&self, snapshot_id: i64) -> HashMap<String, String> {
        let files = self
            .snapshot_store
            .list_files_for_snapshot(snapshot_id)
            .await
            .unwrap_or_default();
        let project_dir = self.project_dir.clone();
        let blobs = self.blobs.clone();
        let max_bytes = self.max_file_bytes();
        tokio::task::spawn_blocking(move || {
            let mut map = HashMap::new();
            for f in files {
                if matches!(
                    f.storage,
                    SnapshotStorage::Deleted | SnapshotStorage::Oversize
                ) {
                    continue;
                }
                if f.size_bytes as u64 > max_bytes {
                    continue;
                }
                let Some(hash) = f.blob_hash.as_deref() else {
                    continue;
                };
                match read_snapshot_content(f.storage, hash, &project_dir, &blobs) {
                    // Skip binary blobs (NUL byte) — gauges read text.
                    Ok(bytes) if !bytes.contains(&0) => {
                        map.insert(f.path, String::from_utf8_lossy(&bytes).into_owned());
                    }
                    _ => continue,
                }
            }
            map
        })
        .await
        .unwrap_or_default()
    }

    /// Resolve the version triple + branch for a snapshot into a run context.
    async fn snapshot_context(
        &self,
        stream_val: i64,
        thread_id: Option<i64>,
        trigger: &'static str,
        snapshot_id: i64,
        subject_default: Option<(String, String)>,
    ) -> GaugeRunContext {
        let version = if snapshot_id > 0 {
            crate::file_ref_version::resolve(&self.snapshot_store, &self.project_dir, snapshot_id)
                .await
                .ok()
        } else {
            None
        };
        GaugeRunContext {
            stream_val,
            thread_id,
            trigger,
            snapshot_id: (snapshot_id > 0).then_some(snapshot_id),
            closest_git_version: version.as_ref().and_then(|v| v.closest_git_version.clone()),
            git_version_exact: version
                .as_ref()
                .map(|v| v.git_version_exact)
                .unwrap_or(false),
            branch: oxplow_git::detect_current_branch(&self.project_dir),
            subject_default,
        }
    }

    /// Run one gauge: build its collector, execute under the sandbox with the
    /// file-map host, and record a run + a sample per `MetricReport.sample`.
    /// Best-effort — errors are logged and swallowed. Returns the sample count.
    async fn run_one_gauge(
        &self,
        metric: &ResolvedMetric,
        ctx: &GaugeRunContext,
        files: Arc<HashMap<String, String>>,
    ) -> usize {
        // Built-in metrics run from their embedded script (no project-disk
        // file); global/project metrics build from their `compute.entryFile`.
        let collector = if metric.scope == "built-in" {
            match builtin_collector(&metric.key) {
                Some(c) => c,
                None => {
                    tracing::warn!(key = %metric.key, "gauge metric: unknown built-in key");
                    return 0;
                }
            }
        } else {
            // A global metric's script lives under the global config dir, not
            // the project (tsk235); project/inline metrics resolve against the
            // project dir.
            let base = self.script_base_dir(metric);
            match compute_to_collector(metric, &base) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(key = %metric.key, error = %e, "gauge metric: bad compute");
                    return 0;
                }
            }
        };
        let source = gauge_source(metric, &collector);
        // The report-derived content (if any); tree-derived gauges ignore it.
        let content = match &metric.compute.report {
            Some(rel) => {
                std::fs::read_to_string(self.script_base_dir(metric).join(rel)).unwrap_or_default()
            }
            None => String::new(),
        };
        let host = GaugeHost::from_shared(files);
        let report =
            match tokio::task::spawn_blocking(move || collector.run_gauge(&content, host)).await {
                Ok(Ok(out)) => out,
                Ok(Err(e)) => {
                    tracing::warn!(key = %metric.key, error = %e, "gauge metric: compute failed");
                    return 0;
                }
                Err(e) => {
                    tracing::warn!(key = %metric.key, error = %e, "gauge metric: join failed");
                    return 0;
                }
            };
        let (samples, gauge_findings, gauge_facts) = match report {
            oxplow_collect_plugin::CollectorOutput::Gauge(r) => (r.samples, r.findings, r.facts),
            _ => return 0,
        };

        let result = async {
            let metric_id = self
                .metrics
                .upsert_definition(metric_definition(metric))
                .await?;
            let mut run = NewMetricRun::done(ctx.stream_val, metric.key.clone(), source.clone());
            run.thread_id = ctx.thread_id;
            run.trigger = Some(ctx.trigger.into());
            run.snapshot_id = ctx.snapshot_id;
            run.closest_git_version = ctx.closest_git_version.clone();
            run.git_version_exact = ctx.git_version_exact;
            run.branch = ctx.branch.clone();
            run.basis_ref = ctx.closest_git_version.clone();

            let mut rows = Vec::new();
            for sample in &samples {
                // A non-finite value (NaN/±inf, e.g. an out-of-range literal in
                // the gauge script) isn't a meaningful measurement — drop it
                // rather than poison the series.
                if !sample.value.is_finite() {
                    continue;
                }
                let (subject_kind, subject_ref) = resolve_subject(&sample.subject, ctx);
                let mut s = NewMetricSample::observed(
                    metric_id,
                    ctx.stream_val,
                    sample.value,
                    source.clone(),
                );
                s.thread_id = ctx.thread_id;
                s.snapshot_id = ctx.snapshot_id;
                s.closest_git_version = ctx.closest_git_version.clone();
                s.git_version_exact = ctx.git_version_exact;
                s.basis_ref = ctx.closest_git_version.clone();
                s.branch = ctx.branch.clone();
                s.subject_kind = subject_kind;
                s.subject_ref = subject_ref;
                s.dims_json = sample
                    .dims
                    .as_ref()
                    .and_then(|d| serde_json::to_string(d).ok());
                rows.push(s);
            }
            // Located items the gauge counted (e.g. each high-complexity
            // function) → `metric_finding` on this run, so a recording drills in.
            let findings: Vec<oxplow_db::NewMetricFinding> = gauge_findings
                .iter()
                .map(|gf| {
                    // Split a `"kind:ref"` subject inline — unlike samples, a
                    // finding never inherits the run's default subject.
                    let (subject_kind, subject_ref) = match &gf.subject {
                        Some(s) => match s.split_once(':') {
                            Some((k, r)) => (Some(k.to_string()), Some(r.to_string())),
                            None => (None, Some(s.clone())),
                        },
                        None => (None, None),
                    };
                    oxplow_db::NewMetricFinding {
                        run_id: 0, // backfilled by record_run_with_data
                        metric_id: Some(metric_id),
                        subject_kind,
                        subject_ref,
                        path: gf.path.clone(),
                        start_line: gf.line,
                        end_line: gf.end_line,
                        col: None,
                        kind: "gauge-item".to_string(),
                        severity: gf.severity.clone(),
                        rule: gf.rule.clone(),
                        message: gf.message.clone(),
                        value: gf.value,
                        extra_json: None,
                    }
                })
                .collect();
            // Atomic: the run, its samples, and its findings commit together.
            let count = rows.len();
            self.metrics
                .record_run_with_data(run, rows, findings)
                .await?;
            Ok::<usize, DomainError>(count)
        }
        .await;

        let count = match result {
            Ok(count) => {
                self.events.emit(OxplowEvent::MetricSamplesChanged {
                    stream_id: StreamId::new(ctx.stream_val),
                });
                count
            }
            Err(e) => {
                tracing::warn!(key = %metric.key, error = %e, "gauge metric: record failed");
                0
            }
        };
        // Inverted substrate (epic tsk12): route the gauge's durable atomic facts
        // into the `fact` layer — dual-written beside the baked sample above until
        // reads flip to the engine. Best-effort; never fails the gauge run.
        self.record_gauge_facts(metric, ctx, &source, &gauge_facts)
            .await;
        count
    }

    /// Persist a gauge's per-item `facts` (epic tsk12) as `fact` rows under one
    /// `metric_capture`, resolving each fact's measure key to a defined measure.
    /// Enforces **declare-to-collect** (decision #4): a fact on an undefined
    /// measure is surfaced via `tracing::warn!` and dropped, never silently
    /// written. No-op without a fact store, with no facts, or with no resolvable
    /// facts. Best-effort — a write error is logged, never propagated.
    async fn record_gauge_facts(
        &self,
        metric: &ResolvedMetric,
        ctx: &GaugeRunContext,
        source: &str,
        gauge_facts: &[oxplow_collect_plugin::GaugeFact],
    ) {
        let Some(facts) = self.fact_store.as_ref() else {
            return;
        };
        if gauge_facts.is_empty() {
            return;
        }
        // Resolve the measure catalog once (one query), then map each fact's key.
        let by_key: HashMap<String, i64> = match facts.list_measures().await {
            Ok(ms) => ms.into_iter().map(|m| (m.key, m.id)).collect(),
            Err(e) => {
                tracing::warn!(key = %metric.key, error = %e, "gauge facts: measure catalog read failed");
                return;
            }
        };
        let mut rows = Vec::new();
        for gf in gauge_facts {
            // A non-finite measurement isn't meaningful — drop it (mirrors the
            // sample guard) rather than poison the fact stream.
            if !gf.value.is_finite() {
                continue;
            }
            let Some(&measure_id) = by_key.get(gf.measure.as_str()) else {
                // Declare-to-collect: a gauge may only emit DEFINED measures.
                tracing::warn!(
                    key = %metric.key, measure = %gf.measure,
                    "gauge facts: undefined measure — fact dropped (declare it in `measures:`)"
                );
                continue;
            };
            let (subject_kind, subject_ref) = match &gf.subject {
                Some(s) => match s.split_once(':') {
                    Some((k, r)) => (Some(k.to_string()), Some(r.to_string())),
                    None => (None, Some(s.clone())),
                },
                None => (None, None),
            };
            rows.push(oxplow_db::NewFact {
                subject_kind,
                subject_ref,
                path: gf.path.clone(),
                line: gf.line,
                dims_json: gf.dims.as_ref().and_then(|d| serde_json::to_string(d).ok()),
                ..oxplow_db::NewFact::new(measure_id, gf.value)
            });
        }
        if rows.is_empty() {
            return;
        }
        let capture = oxplow_db::NewMetricCapture {
            thread_id: ctx.thread_id,
            scope: Some(metric.scope.clone()),
            trigger: Some(ctx.trigger.into()),
            basis_ref: ctx.closest_git_version.clone(),
            snapshot_id: ctx.snapshot_id,
            closest_git_version: ctx.closest_git_version.clone(),
            git_version_exact: ctx.git_version_exact,
            branch: ctx.branch.clone(),
            ..oxplow_db::NewMetricCapture::done(
                ctx.stream_val,
                metric.key.clone(),
                source.to_string(),
            )
        };
        if let Err(e) = facts.record_facts(capture, rows).await {
            tracing::warn!(key = %metric.key, error = %e, "gauge facts: record failed");
        }
    }
}

/// Map a `ResolvedMetric` to a `metric_definition` row (idempotent by key).
fn metric_definition(m: &ResolvedMetric) -> NewMetricDefinition {
    let mut def = NewMetricDefinition::new(m.key.clone(), m.kind.clone(), m.title.clone());
    def.unit = m.unit.clone();
    def.direction = m.direction.clone();
    def.default_agg = m.default_agg.clone();
    def.grain = m.grain.clone();
    def.language = m.language.clone();
    def.description = m.description.clone();
    def.producer = Some(m.key.clone());
    def.category = Some("custom".into());
    def.scope = m.scope.clone();
    def.dimensions_json =
        Some(serde_json::to_string(&m.dimensions).unwrap_or_else(|_| "[]".into()));
    def.target = m.target;
    def.warn_at = m.warn_at;
    def.fail_at = m.fail_at;
    def
}

/// The bundled built-in metric catalog as `MetricEntry` definitions, so the
/// three-scope resolver knows them (a project `use:`s one to activate it). The
/// `compute.entryFile` is a sentinel — built-in collectors run from their
/// embedded script (see [`builtin_collector`]), never a project-disk file.
fn builtin_metric_entries() -> Vec<MetricEntry> {
    builtin_metrics()
        .iter()
        .map(|m| MetricEntry {
            key: Some(m.key.to_string()),
            title: Some(m.title.to_string()),
            kind: Some(m.kind.to_string()),
            unit: Some(m.unit.to_string()),
            direction: Some(m.direction.to_string()),
            grain: Some(m.grain.to_string()),
            // Empty language = a language-agnostic metric (the unified code
            // metrics) — no single language (NULL on the definition).
            language: (!m.language.is_empty()).then(|| m.language.to_string()),
            description: Some(m.description.to_string()),
            dimensions: m.dimensions.iter().map(|d| d.to_string()).collect(),
            target: m.target,
            trigger: Some(m.trigger.to_string()),
            compute: Some(MetricComputeConfig {
                runtime: m.runtime.to_string(),
                input: Some(m.input.to_string()),
                entry_file: Some(m.key.to_string()),
                ..Default::default()
            }),
            ..Default::default()
        })
        .collect()
}

/// The built-in metric SPECS (epic tsk12) for the bundled code gauges — the
/// count-over-facts headlines that replace the baked gauge sample. Each is a
/// `count` over a per-function / per-marker measure; the threshold metrics filter
/// on `min_value`. Because complexity / length are integer measures, strict
/// `> N` in the old gauge equals `>= N+1` here — the equivalence test pins each
/// spec's headline against the baked gauge total so this stays faithful.
fn builtin_metric_specs() -> Vec<NewMetricSpec> {
    fn spec(
        key: &str,
        title: &str,
        measure: &str,
        min_value: Option<f64>,
        direction: &str,
        display_kind: &str,
        description: &str,
    ) -> NewMetricSpec {
        let mut s = NewMetricSpec::base(key, title, measure, "count");
        s.unit = Some("count".into());
        s.filter_json = min_value.map(|v| format!("{{\"min_value\":{v:?}}}"));
        s.direction = direction.into();
        s.display_kind = display_kind.into();
        s.category = Some("static-quality".into());
        s.description = Some(description.into());
        s
    }
    vec![
        spec(
            "oxplow.high_complexity_fns",
            "high-complexity functions",
            "oxplow.complexity",
            Some(11.0), // strict > 10 on an integer measure
            "lower-better",
            "findings",
            "Functions whose cyclomatic complexity exceeds 10 — count over oxplow.complexity facts.",
        ),
        spec(
            "oxplow.long_functions",
            "long functions (>60 lines)",
            "oxplow.fn_length",
            Some(61.0), // strict > 60 on an integer measure
            "lower-better",
            "findings",
            "Functions longer than 60 lines — count over oxplow.fn_length facts.",
        ),
        spec(
            "oxplow.fn_count",
            "function count",
            "oxplow.parameter_count",
            None,
            "neutral",
            "gauge",
            "Total functions / methods defined — count over oxplow.parameter_count facts.",
        ),
        spec(
            "oxplow.todos",
            "TODO / FIXME markers",
            "oxplow.todo",
            None,
            "lower-better",
            "findings",
            "TODO/FIXME/HACK/XXX/BUG markers — count over oxplow.todo facts.",
        ),
    ]
}

/// Build the embedded collector for a built-in metric key, if known.
fn builtin_collector(key: &str) -> Option<Collector> {
    builtin_metrics()
        .iter()
        .find(|m| m.key == key)
        .map(|m| m.collector())
}

/// The starter Starlark gauge written by [`MetricsService::scaffold_metric`].
/// A working tree-derived gauge (counts TODO/FIXME markers across the glob, so
/// it charts immediately and is language-agnostic) with an `ast_query` example
/// in a comment for the author to switch to.
/// A filesystem-safe slug from a namespaced key (non-alphanumerics → `_`), for
/// naming the global scaffold's `<slug>.yaml` file.
fn slugify(key: &str) -> String {
    key.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn starter_metric_script(key: &str, glob: &str, language: Option<&str>) -> String {
    let lang = language.unwrap_or("rust");
    format!(
        "# {key} — a tree-derived gauge. Reads the snapshot via files() and (optionally)\n\
         # the AST via ast_query(); deterministic (no I/O) so samples are `observed`.\n\
         #\n\
         # Edit the body to measure what you want. Starter: count TODO/FIXME markers.\n\
         # To count an AST node instead, e.g.:\n\
         #   n += len(ast_query(f[\"text\"], \"{lang}\", \"(identifier) @x\"))\n\
         def transform(input):\n    \
             n = 0\n    \
             for f in files(\"{glob}\"):\n        \
                 n += len(regex_find(r\"(?i)\\b(TODO|FIXME)\\b\", f[\"text\"]))\n    \
             return {{\"samples\": [{{\"value\": n, \"dims\": {{\"language\": \"{lang}\"}}}}]}}\n"
    )
}

/// Build a gauge [`Collector`] from a metric's `compute:` block (mirrors
/// `collection.rs::plugin_to_collector`, but always `Gauge` kind).
fn compute_to_collector(metric: &ResolvedMetric, project_dir: &Path) -> Result<Collector, String> {
    let c: &MetricComputeConfig = &metric.compute;
    let input = match c.input.as_deref().unwrap_or("text") {
        "text" => CollectorInput::Text,
        "json" => CollectorInput::Json,
        "xml" => CollectorInput::Xml,
        "lcov" => CollectorInput::Lcov,
        "lines" => CollectorInput::Lines,
        other => return Err(format!("unknown input \"{other}\"")),
    };
    let entry_file = c
        .entry_file
        .as_deref()
        .ok_or_else(|| "missing entryFile".to_string())?;
    let abs = project_dir.join(entry_file);
    let name = metric.key.clone();
    let formats = [metric.key.clone()];
    Ok(match c.runtime.as_str() {
        "jaq" | "starlark" => {
            let script = std::fs::read_to_string(&abs)
                .map_err(|e| format!("read entryFile \"{entry_file}\": {e}"))?;
            if c.runtime == "jaq" {
                Collector::jaq(name, CollectorKind::Gauge, formats, input, script)
            } else {
                Collector::starlark(name, CollectorKind::Gauge, formats, input, script)
            }
        }
        "exec" => {
            let mut argv = vec![abs.to_string_lossy().into_owned()];
            argv.extend(c.args.iter().cloned());
            Collector::exec(name, CollectorKind::Gauge, formats, argv)
        }
        other => return Err(format!("unknown runtime \"{other}\"")),
    })
}

/// Trust label: in-process tiers are `observed` under a `metric:<key>` source;
/// the `exec` escape hatch is flagged `plugin-exec:<name>` (lower-trust).
fn gauge_source(metric: &ResolvedMetric, collector: &Collector) -> String {
    use oxplow_collect_plugin::CollectorRuntime;
    if collector.runtime() == CollectorRuntime::Exec {
        format!("plugin-exec:{}", metric.key)
    } else {
        format!("metric:{}", metric.key)
    }
}

/// Split a sample's optional `"kind:ref"` subject onto `(subject_kind,
/// subject_ref)`, falling back to the context's default subject.
fn resolve_subject(
    subject: &Option<String>,
    ctx: &GaugeRunContext,
) -> (Option<String>, Option<String>) {
    match subject {
        Some(s) => match s.split_once(':') {
            Some((kind, rref)) => (Some(kind.to_string()), Some(rref.to_string())),
            None => (None, Some(s.clone())),
        },
        None => match &ctx.subject_default {
            Some((kind, rref)) => (Some(kind.clone()), Some(rref.clone())),
            None => (None, None),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_config::MetricComputeConfig;

    fn init_git_repo(dir: &Path) {
        let repo = git2::Repository::init(dir).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        let sig = repo.signature().unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
    }

    /// A `MetricsService` over a real in-memory `Services` + git repo.
    async fn fixture() -> (Arc<crate::Services>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());
        let svc = Arc::new(crate::Services::in_memory(dir.path()).unwrap());
        svc.streams.ensure_primary().await.unwrap();
        (svc, dir)
    }

    fn starlark_gauge(key: &str, entry_file: &str) -> ResolvedMetric {
        ResolvedMetric {
            key: key.into(),
            title: key.into(),
            kind: "gauge".into(),
            unit: Some("count".into()),
            direction: "lower-better".into(),
            default_agg: "last".into(),
            grain: Some("tree".into()),
            language: Some("rust".into()),
            description: Some("test gauge".into()),
            dimensions: vec!["language".into()],
            target: Some(0.0),
            warn_at: None,
            fail_at: None,
            scope: "project".into(),
            trigger: "on-snapshot".into(),
            compute: MetricComputeConfig {
                runtime: "starlark".into(),
                entry_file: Some(entry_file.into()),
                ..Default::default()
            },
        }
    }

    #[tokio::test]
    async fn run_one_gauge_records_sample_with_version_and_branch() {
        let (svc, dir) = fixture().await;
        // A tree-derived gauge: count unsafe blocks across .rs files.
        std::fs::create_dir_all(dir.path().join("oxplow/metrics")).unwrap();
        std::fs::write(
            dir.path().join("oxplow/metrics/unsafe.star"),
            r#"
def transform(input):
    n = 0
    for f in files("**/*.rs"):
        n += len(ast_query(f["text"], "rust", "(unsafe_block) @u"))
    return {"samples": [{"value": n, "subject": "tree:.", "dims": {"language": "rust"}}]}
"#,
        )
        .unwrap();
        let metric = starlark_gauge("repo.unsafe_blocks", "oxplow/metrics/unsafe.star");

        let mut files = HashMap::new();
        files.insert(
            "src/a.rs".to_string(),
            "fn a() { unsafe { x(); } }\nfn b() { unsafe { y(); } }".to_string(),
        );
        files.insert("src/b.rs".to_string(), "fn c() {}".to_string());

        let ctx = GaugeRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "on-snapshot",
            snapshot_id: Some(42),
            closest_git_version: Some("abc1234".into()),
            git_version_exact: true,
            branch: Some("metrics-substrate".into()),
            subject_default: None,
        };
        let count = svc
            .metrics
            .run_one_gauge(&metric, &ctx, Arc::new(files))
            .await;
        assert_eq!(count, 1);

        let def = svc
            .metric_store
            .get_definition("repo.unsafe_blocks")
            .await
            .unwrap()
            .expect("definition seeded by the run");
        let samples = svc.metric_store.list_samples(def.id).await.unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].value, 2.0, "two unsafe blocks");
        assert_eq!(samples[0].subject_kind.as_deref(), Some("tree"));
        assert_eq!(samples[0].subject_ref.as_deref(), Some("."));
        assert_eq!(samples[0].snapshot_id, Some(42));
        assert_eq!(samples[0].closest_git_version.as_deref(), Some("abc1234"));
        assert_eq!(samples[0].branch.as_deref(), Some("metrics-substrate"));
        assert_eq!(samples[0].source, "metric:repo.unsafe_blocks");
        assert_eq!(
            samples[0].dims_json.as_deref(),
            Some("{\"language\":\"rust\"}")
        );
    }

    #[tokio::test]
    async fn run_one_gauge_records_findings_on_the_run() {
        let (svc, dir) = fixture().await;
        // A gauge that counts long functions AND emits a located finding for each.
        std::fs::create_dir_all(dir.path().join("oxplow/metrics")).unwrap();
        std::fs::write(
            dir.path().join("oxplow/metrics/longfns.star"),
            r#"
def transform(input):
    total = 0
    findings = []
    for f in files("**/*.rs"):
        for m in code_metrics(f["text"], "rust"):
            if m["length"] > 1:
                total += 1
                findings.append({
                    "path": f["path"],
                    "line": m["start_line"],
                    "message": m["name"],
                    "value": m["length"],
                })
    return {"samples": [{"value": total, "subject": "tree:."}], "findings": findings}
"#,
        )
        .unwrap();
        let metric = starlark_gauge("repo.long_fns", "oxplow/metrics/longfns.star");

        let mut files = HashMap::new();
        files.insert(
            "src/a.rs".to_string(),
            "fn big() {\n    let x = 1;\n    let y = 2;\n}\n".to_string(),
        );
        let ctx = GaugeRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "on-snapshot",
            snapshot_id: None,
            closest_git_version: None,
            git_version_exact: false,
            branch: None,
            subject_default: None,
        };
        svc.metrics
            .run_one_gauge(&metric, &ctx, Arc::new(files))
            .await;

        let def = svc
            .metric_store
            .get_definition("repo.long_fns")
            .await
            .unwrap()
            .expect("seeded");
        let samples = svc.metric_store.list_samples(def.id).await.unwrap();
        let run_id = samples[0].run_id.expect("sample has a run");
        let findings = svc.metric_store.list_findings(run_id).await.unwrap();
        assert_eq!(findings.len(), 1, "one long function → one finding");
        assert_eq!(findings[0].path.as_deref(), Some("src/a.rs"));
        assert_eq!(findings[0].message.as_deref(), Some("big"));
        assert_eq!(findings[0].kind, "gauge-item");
        assert!(findings[0].value.unwrap() >= 3.0);
    }

    /// A built-in-scope gauge `ResolvedMetric` — `run_one_gauge` builds its
    /// collector from the embedded script (never a project-disk file).
    fn builtin_gauge(key: &str) -> ResolvedMetric {
        ResolvedMetric {
            key: key.into(),
            title: key.into(),
            kind: "gauge".into(),
            unit: Some("count".into()),
            direction: "lower-better".into(),
            default_agg: "last".into(),
            grain: Some("tree".into()),
            language: None,
            description: None,
            dimensions: vec![],
            target: None,
            warn_at: None,
            fail_at: None,
            scope: "built-in".into(),
            trigger: "on-snapshot".into(),
            compute: MetricComputeConfig::default(),
        }
    }

    /// The old baked headline: the `tree:.` sample of a gauge's definition.
    async fn baked_tree_headline(svc: &crate::Services, key: &str) -> f64 {
        let def = svc
            .metric_store
            .get_definition(key)
            .await
            .unwrap()
            .expect("definition seeded by the run");
        let samples = svc.metric_store.list_samples(def.id).await.unwrap();
        samples
            .iter()
            .find(|s| s.subject_kind.as_deref() == Some("tree"))
            .expect("a tree:. headline sample")
            .value
    }

    /// A mixed-language corpus: a high-complexity + long Rust fn, a TS fn with a
    /// TODO, a Clojure defn with a FIXME. 4 functions; 2 markers; one fn >cc10;
    /// one fn >60 lines.
    fn equivalence_corpus() -> HashMap<String, String> {
        let mut complex = String::from("fn complex(x: i32) -> i32 {\n");
        for i in 0..11 {
            complex.push_str(&format!("    if x == {i} {{ return {i}; }}\n"));
        }
        complex.push_str("    0\n}\n");
        let mut big = String::from("fn big() {\n");
        for i in 0..65 {
            big.push_str(&format!("    let v{i} = {i};\n"));
        }
        big.push_str("}\n");
        let mut files = HashMap::new();
        files.insert("src/c.rs".to_string(), format!("{complex}{big}"));
        files.insert(
            "src/a.ts".to_string(),
            "// TODO wire this up\nfunction f(x: number) { return x; }\n".to_string(),
        );
        files.insert(
            "src/core.clj".to_string(),
            "; FIXME naming\n(defn g [] :ok)\n".to_string(),
        );
        files
    }

    #[tokio::test]
    async fn code_gauge_facts_reaggregate_to_the_baked_headline() {
        // The keystone proof of the inversion (epic tsk12): a metric SPEC computed
        // over the per-item FACTS the gauge emitted == the old baked gauge
        // headline, for every bundled code metric. If this holds, the baked sample
        // is redundant and reads can flip to the engine (tsk26).
        let (svc, _dir) = fixture().await;
        // Seed the built-in specs (count-over-facts) into the catalog.
        svc.metrics.seed_catalog().await;

        let files = Arc::new(equivalence_corpus());
        let ctx = GaugeRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "on-snapshot",
            snapshot_id: Some(7),
            closest_git_version: Some("abc1234".into()),
            git_version_exact: true,
            branch: Some("main".into()),
            subject_default: None,
        };
        for key in [
            "oxplow.fn_count",
            "oxplow.high_complexity_fns",
            "oxplow.long_functions",
            "oxplow.todos",
        ] {
            svc.metrics
                .run_one_gauge(&builtin_gauge(key), &ctx, files.clone())
                .await;
        }

        let engine = crate::metric_engine::MetricEngine::new((*svc.fact_store).clone());
        for (key, expected) in [
            ("oxplow.fn_count", 4.0),
            ("oxplow.high_complexity_fns", 1.0),
            ("oxplow.long_functions", 1.0),
            ("oxplow.todos", 2.0),
        ] {
            let baked = baked_tree_headline(&svc, key).await;
            assert_eq!(baked, expected, "{key}: baked headline");
            let spec = svc
                .fact_store
                .get_spec(key)
                .await
                .unwrap()
                .unwrap_or_else(|| panic!("{key} spec seeded"));
            let engine_headline = engine.headline_for_spec(&spec).await.unwrap();
            assert_eq!(
                engine_headline,
                Some(baked),
                "{key}: facts re-aggregated through the engine must equal the baked headline",
            );
        }
    }

    #[tokio::test]
    async fn gauge_facts_on_undefined_measure_are_dropped_not_written() {
        // Declare-to-collect (decision #4): a gauge may only emit DEFINED measures.
        // A fact on an undefined measure is dropped (surfaced via warn), while a
        // sibling fact on a defined measure in the same report still lands.
        let (svc, dir) = fixture().await;
        std::fs::create_dir_all(dir.path().join("oxplow/metrics")).unwrap();
        std::fs::write(
            dir.path().join("oxplow/metrics/mixed.star"),
            r#"
def transform(input):
    return {"samples": [{"value": 2, "subject": "tree:."}], "facts": [
        {"measure": "oxplow.complexity", "value": 5, "subject": "symbol:src/a.rs::foo"},
        {"measure": "acme.undefined", "value": 9, "subject": "symbol:src/a.rs::bar"},
    ]}
"#,
        )
        .unwrap();
        let metric = starlark_gauge("acme.mixed_facts", "oxplow/metrics/mixed.star");
        let ctx = GaugeRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "manual",
            snapshot_id: None,
            closest_git_version: None,
            git_version_exact: false,
            branch: None,
            subject_default: None,
        };
        svc.metrics
            .run_one_gauge(&metric, &ctx, Arc::new(HashMap::new()))
            .await;

        // The defined-measure fact landed…
        let complexity = svc
            .fact_store
            .get_measure("oxplow.complexity")
            .await
            .unwrap()
            .unwrap();
        let rows = svc
            .fact_store
            .facts_for_measure(complexity.id)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "the defined-measure fact is written");
        assert_eq!(rows[0].value, 5.0);
        // …and the undefined measure was never auto-created by the write.
        assert!(
            svc.fact_store
                .get_measure("acme.undefined")
                .await
                .unwrap()
                .is_none(),
            "an undefined measure is not conjured by a dropped fact"
        );
    }

    #[tokio::test]
    async fn seed_definitions_upserts_configured_metrics() {
        let (svc, dir) = fixture().await;
        std::fs::write(
            oxplow_config::config_path(dir.path()),
            "metrics:\n  - key: repo.loc\n    kind: gauge\n    title: \"lines\"\n    compute: { runtime: starlark, entryFile: m.star }\n",
        )
        .unwrap();
        svc.reload_config_from_disk().unwrap();
        let n = svc.metrics.seed_definitions().await;
        assert_eq!(n, 1);
        let def = svc
            .metric_store
            .get_definition("repo.loc")
            .await
            .unwrap()
            .expect("seeded");
        assert_eq!(def.kind, "gauge");
        assert_eq!(def.scope, "project");
        assert_eq!(def.producer.as_deref(), Some("repo.loc"));
    }

    #[tokio::test]
    async fn seed_catalog_upserts_configured_measures_and_dimensions() {
        let (svc, dir) = fixture().await;
        std::fs::write(
            oxplow_config::config_path(dir.path()),
            "measures:\n  - key: acme.api_latency\n    unit: ms\n    \
             temporalSemantics: non-additive\ndimensions:\n  - key: acme.endpoint\n    \
             label: Endpoint\n    vocabulary: [list, get]\n",
        )
        .unwrap();
        svc.reload_config_from_disk().unwrap();
        let (m, d) = svc.metrics.seed_catalog().await;
        assert_eq!(
            (m, d),
            (1, 1),
            "one project measure + one project dimension"
        );

        // The custom measure lands beside the migration-seeded `oxplow.*` built-ins.
        let measure = svc
            .fact_store
            .get_measure("acme.api_latency")
            .await
            .unwrap()
            .expect("measure seeded");
        assert_eq!(measure.scope, "project");
        assert_eq!(measure.unit.as_deref(), Some("ms"));
        assert_eq!(measure.temporal_semantics, "non-additive");

        let dims = svc.fact_store.list_dimensions().await.unwrap();
        let ep = dims
            .iter()
            .find(|d| d.key == "acme.endpoint")
            .expect("dimension seeded");
        assert_eq!(ep.scope, "project");
        assert_eq!(ep.vocabulary_json.as_deref(), Some("[\"list\",\"get\"]"));
    }

    #[tokio::test]
    async fn scaffold_measure_writes_config_and_seeds_catalog() {
        let (svc, dir) = fixture().await;
        let key = svc
            .metrics
            .scaffold_measure(
                "acme.api_latency",
                Some("API latency".into()),
                Some("ms".into()),
                Some("endpoint".into()),
                Some("non-additive".into()),
                None,
            )
            .await
            .unwrap();
        assert_eq!(key, "acme.api_latency");
        // Persisted to project.yaml AND seeded into the catalog (scaffold reseeds).
        let raw = std::fs::read_to_string(oxplow_config::config_path(dir.path())).unwrap();
        assert!(raw.contains("acme.api_latency"), "got:\n{raw}");
        let m = svc
            .fact_store
            .get_measure("acme.api_latency")
            .await
            .unwrap()
            .expect("seeded");
        assert_eq!(m.unit.as_deref(), Some("ms"));
        assert_eq!(m.temporal_semantics, "non-additive");
        // Reserved namespace + duplicate both rejected.
        assert!(svc
            .metrics
            .scaffold_measure("oxplow.x", None, None, None, None, None)
            .await
            .is_err());
        assert!(svc
            .metrics
            .scaffold_measure("acme.api_latency", None, None, None, None, None)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn scaffold_dimension_writes_config_and_seeds_catalog() {
        let (svc, dir) = fixture().await;
        let key = svc
            .metrics
            .scaffold_dimension(
                "acme.license",
                Some("License".into()),
                Some("categorical".into()),
                None,
            )
            .await
            .unwrap();
        assert_eq!(key, "acme.license");
        let raw = std::fs::read_to_string(oxplow_config::config_path(dir.path())).unwrap();
        assert!(raw.contains("acme.license"), "got:\n{raw}");
        let dims = svc.fact_store.list_dimensions().await.unwrap();
        assert!(dims
            .iter()
            .any(|d| d.key == "acme.license" && d.scope == "project"));
        assert!(svc
            .metrics
            .scaffold_dimension("oxplow.x", None, None, None)
            .await
            .is_err());
        assert!(svc
            .metrics
            .scaffold_dimension("acme.license", None, None, None)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn used_builtin_resolves_at_builtin_scope_and_runs_embedded() {
        let (svc, dir) = fixture().await;
        // A user enables a bundled built-in by `use:` — no project script.
        std::fs::write(
            oxplow_config::config_path(dir.path()),
            "metrics:\n  - use: oxplow.rust.unsafe_blocks\n    target: 3\n",
        )
        .unwrap();
        svc.reload_config_from_disk().unwrap();

        // Seeding registers the definition at built-in scope, with the project's
        // target override applied.
        assert_eq!(svc.metrics.seed_definitions().await, 1);
        let def = svc
            .metric_store
            .get_definition("oxplow.rust.unsafe_blocks")
            .await
            .unwrap()
            .expect("seeded");
        assert_eq!(def.scope, "built-in");
        assert_eq!(def.language.as_deref(), Some("rust"));
        assert_eq!(def.target, Some(3.0), "project override merged");

        // Running it executes the EMBEDDED script (no project-disk file). With
        // no snapshot the file map is empty, so it cleanly yields a 0 sample.
        let count = svc
            .metrics
            .run_metric_by_key("oxplow.rust.unsafe_blocks", None)
            .await
            .unwrap();
        assert_eq!(count, 1);
        let samples = svc.metric_store.list_samples(def.id).await.unwrap();
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].value, 0.0);
        assert_eq!(samples[0].source, "metric:oxplow.rust.unsafe_blocks");
    }

    #[tokio::test]
    async fn catalog_lists_builtins_and_enable_toggle_writes_config() {
        let (svc, dir) = fixture().await;

        // Built-ins appear in the catalog, not enabled until `use:`d.
        let cat = svc.metrics.catalog().await;
        assert!(!cat.is_empty(), "built-in catalog is non-empty");
        let entry = cat
            .iter()
            .find(|e| e.key == "oxplow.rust.unsafe_blocks")
            .expect("built-in present");
        assert_eq!(entry.scope, "built-in");
        assert!(!entry.enabled, "not enabled before use:");
        assert!(entry.toggleable, "code gauges are toggleable");
        assert_eq!(entry.category.as_deref(), Some("custom"));

        // Enable end-to-end: writes a `use:` into .oxplow/project.yaml + seeds the def.
        svc.metrics
            .set_metric_enabled("oxplow.rust.unsafe_blocks", true)
            .await
            .unwrap();
        assert!(
            svc.metrics
                .catalog()
                .await
                .iter()
                .find(|e| e.key == "oxplow.rust.unsafe_blocks")
                .unwrap()
                .enabled,
            "now enabled"
        );
        let def = svc
            .metric_store
            .get_definition("oxplow.rust.unsafe_blocks")
            .await
            .unwrap()
            .expect("definition seeded on enable");
        assert_eq!(def.scope, "built-in");
        let yaml = std::fs::read_to_string(oxplow_config::config_path(dir.path())).unwrap();
        assert!(
            yaml.contains("oxplow.rust.unsafe_blocks"),
            "use: persisted to .oxplow/project.yaml; got:\n{yaml}"
        );

        // Disable removes it from config.
        svc.metrics
            .set_metric_enabled("oxplow.rust.unsafe_blocks", false)
            .await
            .unwrap();
        assert!(
            !svc.metrics
                .catalog()
                .await
                .iter()
                .find(|e| e.key == "oxplow.rust.unsafe_blocks")
                .unwrap()
                .enabled,
            "disabled again"
        );
    }

    #[tokio::test]
    async fn catalog_lists_all_producer_metrics_before_any_data() {
        // The Catalog is a registry: every always-on producer metric must be
        // visible even on a brand-new project with zero recorded samples (tsk286).
        let (svc, _dir) = fixture().await;
        let cat = svc.metrics.catalog().await;
        let by_key: std::collections::HashMap<&str, &MetricCatalogEntry> =
            cat.iter().map(|e| (e.key.as_str(), e)).collect();

        for (key, kind, category) in [
            ("oxplow.coverage.abs_pct", "coverage", "coverage"),
            ("oxplow.tests.passed", "gauge", "testing"),
            ("oxplow.analysis.errors", "gauge", "static-quality"),
            ("agent.tokens.total", "gauge", "operational"),
            ("agent.nudges.fired", "event", "operational"),
            ("effort.cycle_time_ms", "gauge", "operational"),
        ] {
            let e = by_key
                .get(key)
                .unwrap_or_else(|| panic!("{key} listed in catalog with no data"));
            assert_eq!(e.kind, kind, "{key} kind");
            assert_eq!(e.category.as_deref(), Some(category), "{key} category");
            assert!(!e.toggleable, "{key} is always-on, not toggleable");
        }
        // Toggleable code gauges still coexist.
        assert!(
            by_key
                .get("oxplow.rust.unsafe_blocks")
                .is_some_and(|e| e.toggleable),
            "code gauges present + toggleable"
        );
    }

    #[tokio::test]
    async fn catalog_unions_always_on_producer_definitions() {
        let (svc, _dir) = fixture().await;

        // Simulate a producer (or external plugin) seeding a definition directly,
        // the way token-parse / tests / coverage do at runtime.
        let mut def = NewMetricDefinition::new(
            "agent.tokens.total".to_string(),
            "gauge".to_string(),
            "Total tokens".to_string(),
        );
        def.category = Some("operational".to_string());
        def.producer = Some("token-parse".to_string());
        svc.metric_store.upsert_definition(def).await.unwrap();

        let cat = svc.metrics.catalog().await;
        let entry = cat
            .iter()
            .find(|e| e.key == "agent.tokens.total")
            .expect("producer-seeded metric is in the catalog");
        assert!(!entry.toggleable, "always-on producers are not toggleable");
        assert!(entry.enabled, "always-on producers read as enabled");
        assert_eq!(entry.category.as_deref(), Some("operational"));

        // A toggleable code gauge still coexists in the same listing.
        assert!(
            cat.iter()
                .any(|e| e.key == "oxplow.rust.unsafe_blocks" && e.toggleable),
            "code gauges still present and toggleable"
        );
    }

    #[tokio::test]
    async fn set_metric_override_writes_target_trigger_stays_inherent() {
        let (svc, dir) = fixture().await;

        // Setting a target override on a not-yet-enabled metric enables it and
        // persists the target into .oxplow/project.yaml.
        svc.metrics
            .set_metric_override("oxplow.rust.unsafe_blocks", Some(0.0))
            .await
            .unwrap();

        let entry = svc
            .metrics
            .catalog()
            .await
            .into_iter()
            .find(|e| e.key == "oxplow.rust.unsafe_blocks")
            .unwrap();
        assert!(entry.enabled, "override implies enabled");
        assert_eq!(entry.target, Some(0.0));
        // Trigger is inherent to the definition (on-snapshot for code gauges),
        // never overridable (tsk290).
        assert_eq!(entry.trigger, "on-snapshot");

        let yaml = std::fs::read_to_string(oxplow_config::config_path(dir.path())).unwrap();
        assert!(yaml.contains("target"), "target persisted; got:\n{yaml}");
        assert!(
            !yaml.contains("trigger"),
            "trigger is never written to config; got:\n{yaml}"
        );

        // Clearing the target override (None) drops it back to the default.
        svc.metrics
            .set_metric_override("oxplow.rust.unsafe_blocks", None)
            .await
            .unwrap();
        let entry = svc
            .metrics
            .catalog()
            .await
            .into_iter()
            .find(|e| e.key == "oxplow.rust.unsafe_blocks")
            .unwrap();
        // unsafe_blocks ships with target 0 in the built-in catalog, so the
        // resolved target falls back to that default, not the cleared override.
        assert_eq!(entry.target, Some(0.0));
    }

    #[tokio::test]
    async fn scaffold_metric_writes_script_entry_and_seeds() {
        let (svc, dir) = fixture().await;

        let rel = svc
            .metrics
            .scaffold_metric(
                "acme.todo_density",
                Some("TODO density".to_string()),
                Some("rust".to_string()),
                Some("**/*.rs".to_string()),
                None,
            )
            .await
            .unwrap();
        assert_eq!(rel, "oxplow/metrics/acme_todo_density.star");

        // Script stub written + uses the public capability surface.
        let script = std::fs::read_to_string(dir.path().join(&rel)).unwrap();
        assert!(script.contains("def transform(input):"), "got:\n{script}");
        assert!(
            script.contains("files(\"**/*.rs\")"),
            "glob threaded; got:\n{script}"
        );

        // metrics: key entry persisted with the compute block.
        let yaml = std::fs::read_to_string(oxplow_config::config_path(dir.path())).unwrap();
        assert!(
            yaml.contains("acme.todo_density"),
            "key persisted; got:\n{yaml}"
        );
        assert!(
            yaml.contains("acme_todo_density.star"),
            "entryFile persisted; got:\n{yaml}"
        );

        // Definition seeded as a project-scoped gauge.
        let def = svc
            .metric_store
            .get_definition("acme.todo_density")
            .await
            .unwrap()
            .expect("scaffolded definition seeded");
        assert_eq!(def.kind, "gauge");
        assert_eq!(def.scope, "project");

        // Scaffolding the same key again is rejected (no duplicate entry).
        assert!(svc
            .metrics
            .scaffold_metric("acme.todo_density", None, None, None, None)
            .await
            .is_err());
        // Reserved namespace is rejected.
        assert!(svc
            .metrics
            .scaffold_metric("oxplow.nope", None, None, None, None)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn scaffold_metric_global_writes_to_global_dir_and_seeds() {
        let (svc, _dir) = fixture().await;
        let gtmp = tempfile::tempdir().unwrap();
        // A handle pointed at an isolated global dir (no env race).
        let m = svc
            .metrics
            .clone()
            .with_global_dir(gtmp.path().to_path_buf());

        let path = m
            .scaffold_metric(
                "myglobal.todo",
                Some("Global TODO".to_string()),
                None,
                Some("**/*".to_string()),
                Some("global".to_string()),
            )
            .await
            .unwrap();

        // Script + manifest written under <global>/metrics/, not the project.
        assert!(std::path::Path::new(&path).exists(), "script at {path}");
        assert!(path.ends_with("metrics/myglobal_todo.star"), "got {path}");
        let manifest =
            std::fs::read_to_string(gtmp.path().join("metrics/myglobal_todo.yaml")).unwrap();
        assert!(manifest.contains("myglobal.todo"), "got:\n{manifest}");
        assert!(manifest.contains("myglobal_todo.star"), "got:\n{manifest}");

        // Seeded at scope `global` (the resolver read it from the global dir).
        let entry = m
            .catalog()
            .await
            .into_iter()
            .find(|e| e.key == "myglobal.todo")
            .expect("global metric in catalog");
        assert_eq!(entry.scope, "global");
        assert!(entry.enabled);

        // A second scaffold of the same global key is rejected.
        assert!(m
            .scaffold_metric(
                "myglobal.todo",
                None,
                None,
                None,
                Some("global".to_string())
            )
            .await
            .is_err());
    }

    #[tokio::test]
    async fn run_one_gauge_resolves_global_scope_script_from_global_dir() {
        let (svc, _dir) = fixture().await;
        let gtmp = tempfile::tempdir().unwrap();
        // The global metric's script lives under <global>/metrics/, NOT the
        // project — the project dir has no such file.
        std::fs::create_dir_all(gtmp.path().join("metrics")).unwrap();
        std::fs::write(
            gtmp.path().join("metrics/g.star"),
            "def transform(input):\n    return {\"samples\": [{\"value\": float(len(files(\"**/*\")))}]}\n",
        )
        .unwrap();

        let mut metric = starlark_gauge("myglobal.filecount", "g.star");
        metric.scope = "global".into();

        let m = svc
            .metrics
            .clone()
            .with_global_dir(gtmp.path().to_path_buf());
        let mut files = HashMap::new();
        files.insert("a.rs".to_string(), "x".to_string());
        files.insert("b.rs".to_string(), "y".to_string());
        let ctx = GaugeRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "on-snapshot",
            snapshot_id: None,
            closest_git_version: None,
            git_version_exact: false,
            branch: None,
            subject_default: None,
        };
        let count = m.run_one_gauge(&metric, &ctx, Arc::new(files)).await;
        assert_eq!(count, 1, "global-scope script resolved + ran");
        let def = svc
            .metric_store
            .get_definition("myglobal.filecount")
            .await
            .unwrap()
            .unwrap();
        let samples = svc.metric_store.list_samples(def.id).await.unwrap();
        assert_eq!(samples[0].value, 2.0, "counted both files");
    }

    #[test]
    fn resolve_subject_splits_kind_ref_and_falls_back() {
        let ctx = GaugeRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "manual",
            snapshot_id: None,
            closest_git_version: None,
            git_version_exact: false,
            branch: None,
            subject_default: Some(("effort".into(), "eff7".into())),
        };
        assert_eq!(
            resolve_subject(&Some("file:src/a.rs".into()), &ctx),
            (Some("file".into()), Some("src/a.rs".into()))
        );
        // No colon → bare ref.
        assert_eq!(
            resolve_subject(&Some("whole".into()), &ctx),
            (None, Some("whole".into()))
        );
        // None → context default.
        assert_eq!(
            resolve_subject(&None, &ctx),
            (Some("effort".into()), Some("eff7".into()))
        );
    }
}
