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

use oxplow_collect_plugin::{
    builtin_metrics, Collector, CollectorInput, CollectorKind, GaugeHost, SandboxBudget,
};
use oxplow_config::{
    global_config_dir, load_global_dimension_entries, load_global_gauge_entries,
    load_global_measure_entries, load_global_metric_entries, resolve_dimensions, resolve_gauges,
    resolve_measures, resolve_metrics, DimensionEntry, GaugeComputeConfig, GaugeEntry,
    MeasureEntry, MetricEntry, OxplowConfig, ResolvedGauge, ResolvedSpec,
};
use oxplow_db::{
    NewDimension, NewMeasure, NewMetricSpec, SnapshotStorage, SqliteFactStore, SqliteSnapshotStore,
    SqliteTaskEffortStore, SqliteThreadStore, TaskEffortStore,
};
use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{EffortId, StreamId, ThreadId};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::blob_store::BlobStore;
use crate::events::{EventBus, OxplowEvent, SnapshotSourceKind};
use crate::producer_metrics::builtin_producer_metrics;
use crate::snapshot_content::read_snapshot_content;

const DEFAULT_MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;

/// Wall-clock ceiling for ONE gauge run (tsk47).
///
/// The `SandboxBudget` default is 5s, which suits a report parser reading a single
/// file. A tree gauge is a different animal: it tree-sitter-parses the WHOLE tree
/// (873 files here) on a full-tree baseline, and 5s was nowhere near enough — the
/// broad-query gauges silently timed out on every full run, so `oxplow.ts.console_calls`
/// and `oxplow.ts.ts_ignore` had produced ZERO facts since the project was indexed.
///
/// Gauges run detached on a blocking thread, so a generous ceiling costs nothing in
/// latency; it exists only to catch a genuinely runaway script.
const GAUGE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// A gauge sweep over at least this many files is a WHOLE-TREE sweep (the baseline)
/// rather than an ordinary per-commit delta, so it gets tracked as a visible
/// background task (tsk48). A delta is a handful of files and finishes in
/// milliseconds; tracking those would just be noise.
const TREE_SWEEP_FILE_THRESHOLD: usize = 100;

/// What a gauge sweep did — how many gauges actually ran (after the idempotency
/// skip) and which failed. Returned so [`crate::Services::rebuild_metric_baseline`]
/// can report the outcome to an MCP caller or a test instead of it vanishing into a
/// background-task label (tsk50).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SweepReport {
    pub ran: usize,
    pub failed: Vec<String>,
}

/// Runs config-declared metrics into the substrate. Cheap to clone (a handle of
/// leaf `Arc`s) — deliberately NOT holding `Arc<Services>`, to avoid a cycle.
/// The four global-scope catalog blocks parsed from `<global_dir>/{metrics,
/// gauges,measures,dimensions}/*.yaml`. Cached so the hot read paths
/// (`resolved_specs`/`resolved_gauges`, run on every snapshot event) don't
/// re-read + re-parse these files each time (tsk17). Project config stays read
/// fresh from the in-memory `RwLock` — only the *disk* loads are cached.
#[derive(Default)]
struct GlobalCatalog {
    metrics: Vec<MetricEntry>,
    gauges: Vec<GaugeEntry>,
    measures: Vec<MeasureEntry>,
    dimensions: Vec<DimensionEntry>,
}

#[derive(Clone)]
pub struct MetricsService {
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
    /// Background-task store, so a whole-tree gauge sweep is VISIBLE while it runs
    /// (tsk48). `None` in tests. Wired at boot via [`Self::with_background_tasks`].
    background_tasks: Option<crate::background_task::BackgroundTaskStore>,
    /// Lazily-loaded cache of the global-scope catalog files (tsk17). Cleared on
    /// every in-app `ConfigChanged` emit, so an in-app scaffold/toggle reflects
    /// immediately; an *external* edit to a global YAML needs any in-app config
    /// op to refresh (global files aren't watched — the same semantics as
    /// before, minus the per-event disk read). `Arc` so clones of the service
    /// share one cache (and its invalidations).
    global_catalog: Arc<std::sync::Mutex<Option<GlobalCatalog>>>,
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
    /// The producing effort, when the trigger knows it unambiguously (the
    /// `on-effort-complete` ride-along) — stamped onto the capture so the
    /// effort-attribution read (`captures_for_effort`) sees the run (tsk43).
    /// Snapshot/manual scans are effort-less (`None`).
    effort_id: Option<i64>,
    /// The capture's scanned-set semantics (tsk71): `delta` for the ordinary
    /// incremental rescan over the snapshot's own rows; `full` for a baseline
    /// over the RECONSTRUCTED tree as-of the snapshot. Stamped verbatim onto
    /// the capture — the per-path fold branches on it.
    scan_kind: &'static str,
}

impl MetricsService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        snapshot_store: Arc<SqliteSnapshotStore>,
        thread_store: Arc<SqliteThreadStore>,
        effort_store: Arc<SqliteTaskEffortStore>,
        blobs: BlobStore,
        config: Arc<RwLock<OxplowConfig>>,
        project_dir: PathBuf,
        events: EventBus,
    ) -> Self {
        Self {
            snapshot_store,
            thread_store,
            effort_store,
            blobs,
            config,
            project_dir,
            events,
            global_dir: None,
            fact_store: None,
            background_tasks: None,
            global_catalog: Arc::new(std::sync::Mutex::new(None)),
        }
    }

    /// Override the global config dir (test seam; default `global_config_dir()`).
    pub fn with_global_dir(mut self, dir: PathBuf) -> Self {
        self.global_dir = Some(dir);
        // Changing the source dir must not serve the shared cache's entries from
        // the old dir — give this handle a fresh cache (tsk17).
        self.global_catalog = Arc::new(std::sync::Mutex::new(None));
        self
    }

    /// Wire the fact substrate so `run()` seeds config-declared measures +
    /// dimensions into the catalog beside the migration-seeded built-ins.
    /// Wire the background-task store so full-tree gauge sweeps report progress.
    pub fn with_background_tasks(
        mut self,
        tasks: crate::background_task::BackgroundTaskStore,
    ) -> Self {
        self.background_tasks = Some(tasks);
        self
    }

    pub fn with_fact_store(mut self, fact_store: Arc<SqliteFactStore>) -> Self {
        self.fact_store = Some(fact_store);
        self
    }

    /// The effective global config dir (the field override, else the platform
    /// `global_config_dir()`); `metrics/` hangs under it.
    fn effective_global_dir(&self) -> Option<PathBuf> {
        self.global_dir.clone().or_else(global_config_dir)
    }

    /// Run `f` against the cached global catalog, loading it from disk once on
    /// first use (tsk17). The hot read paths call this instead of re-reading the
    /// four global YAML dirs every time.
    fn with_global_catalog<R>(&self, f: impl FnOnce(&GlobalCatalog) -> R) -> R {
        let mut guard = self
            .global_catalog
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if guard.is_none() {
            let dir = self.effective_global_dir();
            *guard = Some(match dir {
                Some(d) => GlobalCatalog {
                    metrics: load_global_metric_entries(&d),
                    gauges: load_global_gauge_entries(&d),
                    measures: load_global_measure_entries(&d),
                    dimensions: load_global_dimension_entries(&d),
                },
                None => GlobalCatalog::default(),
            });
        }
        f(guard.as_ref().expect("populated above"))
    }

    /// Drop the cached global catalog so the next read reloads it. Called on
    /// every in-app config mutation (beside each `ConfigChanged` emit).
    fn invalidate_global_catalog(&self) {
        *self
            .global_catalog
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = None;
    }

    /// Base dir a gauge's `compute.entryFile` / `report` resolves against:
    /// `<global>/gauges` for a global-scope gauge, else the project dir. Falls
    /// back to the project dir if no global dir is available.
    fn script_base_dir(&self, gauge: &ResolvedGauge) -> PathBuf {
        if gauge.scope == "global" {
            if let Some(g) = self.effective_global_dir() {
                return g.join("gauges");
            }
        }
        self.project_dir.clone()
    }

    /// The active, resolved metric SPECS for this project (built-in ∪ global ∪
    /// project, precedence project > global > built-in). Built-ins are the
    /// bundled catalog (`oxplow_collect_plugin::builtin_metrics`); a project
    /// activates one with `metrics: - use: oxplow.<lang>.<name>` and its own
    /// `key:` specs.
    fn resolved_specs(&self) -> Vec<ResolvedSpec> {
        let project = self
            .config
            .read()
            .map(|c| c.metrics.clone())
            .unwrap_or_default();
        let global = self.with_global_catalog(|g| g.metrics.clone());
        let builtin = builtin_spec_entries();
        resolve_metrics(&builtin, &global, &project)
    }

    /// The active, resolved GAUGES (fact producers) for this project: config
    /// `gauges:` (global ∪ project, always active once declared) ∪ the built-in
    /// gauges whose metric is `use:`-enabled in this project. This is what the
    /// run paths execute.
    fn resolved_gauges(&self) -> Vec<ResolvedGauge> {
        let project = self
            .config
            .read()
            .map(|c| c.gauges.clone())
            .unwrap_or_default();
        let global = self.with_global_catalog(|g| g.gauges.clone());
        let mut out = resolve_gauges(&global, &project);
        // Built-in gauges run only when their metric is enabled (`metrics: use:`)
        // AND not disabled by a marker — a disabled gauge must not compute.
        let enabled: std::collections::HashSet<String> = self
            .resolved_specs()
            .into_iter()
            .filter(|s| s.scope == "built-in" && s.enabled)
            .map(|s| s.key)
            .collect();
        for m in builtin_metrics() {
            if enabled.contains(m.key) {
                out.push(builtin_gauge(m.key, m.trigger));
            }
        }
        out
    }

    fn max_file_bytes(&self) -> u64 {
        self.config
            .read()
            .map(|c| c.snapshot_max_file_bytes)
            .unwrap_or(DEFAULT_MAX_FILE_BYTES)
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
        let (global_measures, global_dims) =
            self.with_global_catalog(|g| (g.measures.clone(), g.dimensions.clone()));

        let mut m = 0;
        for rm in resolve_measures(&global_measures, &project_measures) {
            // `rm.component_role` is intentionally not forwarded — the measure
            // row's `component_role` is a dead column (tsk15).
            let nm = NewMeasure {
                key: rm.key.clone(),
                title: rm.title,
                unit: rm.unit,
                subject_kind: rm.subject_kind,
                temporal_semantics: rm.temporal_semantics,
                capture_scope: rm.capture_scope,
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
                promoted: rd.promote,
            };
            match facts.upsert_dimension(nd).await {
                Ok(()) => d += 1,
                Err(e) => tracing::warn!(key = %rd.key, error = %e, "failed to seed dimension"),
            }
        }
        // Metric SPECS — RECONCILE the `metric_spec` table down to exactly the
        // *enabled* set (tsk31). Because reads treat a missing spec as empty and
        // producers gate collection on `measure_has_active_spec`, pruning a
        // disabled metric's row is the single lever that hides it AND stops its
        // base-data collection. Per-metric enabled state comes from config:
        let cfg_metrics = self
            .config
            .read()
            .map(|c| c.metrics.clone())
            .unwrap_or_default();
        // `None` = no config entry; `Some(true/false)` = an explicit flag.
        let config_state = |key: &str| -> Option<bool> {
            cfg_metrics
                .iter()
                .find(|e| e.use_key.as_deref() == Some(key) || e.key.as_deref() == Some(key))
                .map(|e| e.enabled.unwrap_or(true))
        };
        // Built-in metric SPECS — the bundled code/idiom gauge specs
        // (`builtin_metric_specs` + `builtin_ast_specs`) and the always-on producer
        // specs. All are seeded UNLESS explicitly disabled by a `enabled: false`
        // marker in config, in which case the row is pruned (so spec-driven reads
        // go empty and, for producers, `measure_has_active_spec` closes the
        // collection gate). Built-in gauges keep their spec seeded when merely
        // un-`use:`d — the gauge simply doesn't RUN (gated in `resolved_gauges`) —
        // so a disable is only ever an explicit marker.
        for spec in builtin_metric_specs()
            .into_iter()
            .chain(builtin_ast_specs())
            .chain(crate::producer_metrics::builtin_producer_specs())
        {
            let key = spec.key.clone();
            let res = if config_state(&key) != Some(false) {
                facts.upsert_spec(spec).await.map(|_| ())
            } else {
                facts.delete_spec(&key).await
            };
            if let Err(e) = res {
                tracing::warn!(key = %key, error = %e, "failed to reconcile built-in metric spec");
            }
        }
        // Config-declared SPECS (global ∪ project `metrics:`). A `key:` seeds a new
        // spec; a `use:` of a BUILT-IN resolves to scope `built-in` carrying the
        // catalog default target plus the project's threshold overrides (the
        // Catalog inline target editor writes exactly such a `use:`), so it must
        // re-seed AFTER the override-free built-ins above — dropping it left
        // target/warn_at/fail_at NULL everywhere the engine reads the spec row. A
        // disabled entry (`enabled: false`) is pruned instead of seeded.
        for s in self.resolved_specs() {
            let res = if s.enabled {
                facts.upsert_spec(spec_to_new_spec(&s)).await.map(|_| ())
            } else {
                facts.delete_spec(&s.key).await
            };
            if let Err(e) = res {
                tracing::warn!(key = %s.key, error = %e, "failed to reconcile config metric spec");
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
        let resolved = self.resolved_specs();
        let by_key: std::collections::HashMap<&str, &_> =
            resolved.iter().map(|m| (m.key.as_str(), m)).collect();
        // Per-key config enabled state (tsk31): `None` = no entry, `Some(_)` = an
        // explicit flag. Producers/plugins are default-ON (a disable marker turns
        // them off); built-in gauges are default-OFF (a `use:` turns them on).
        let cfg_metrics = self
            .config
            .read()
            .map(|c| c.metrics.clone())
            .unwrap_or_default();
        let config_state = |key: &str| -> Option<bool> {
            cfg_metrics
                .iter()
                .find(|e| e.use_key.as_deref() == Some(key) || e.key.as_deref() == Some(key))
                .map(|e| e.enabled.unwrap_or(true))
        };
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for b in builtin_metrics() {
            seen.insert(b.key.to_string());
            // When enabled, surface the *resolved* target (so a project override
            // shows through, tsk233); otherwise the built-in defaults. Trigger is a
            // property of the built-in gauge, not overridable. A built-in gauge is
            // enabled only when a (non-disabled) `use:` resolves it.
            let r = by_key.get(b.key);
            out.push(MetricCatalogEntry {
                key: b.key.to_string(),
                title: b.title.to_string(),
                kind: b.kind.to_string(),
                language: Some(b.language.to_string()),
                scope: "built-in".to_string(),
                enabled: r.is_some_and(|m| m.enabled),
                target: r.map_or(b.target, |m| m.target),
                trigger: b.trigger.to_string(),
                toggleable: true,
                // Match the seeded spec's category (builtin_metric_specs /
                // builtin_ast_specs seed "static-quality"), letting a resolved
                // config override win — the Catalog must agree with the spec
                // catalog it toggles (tsk46).
                category: r
                    .and_then(|m| m.category.clone())
                    .or_else(|| Some("static-quality".to_string())),
            });
        }
        // Project/global-defined metric specs not already shown as a built-in.
        for m in &resolved {
            if seen.insert(m.key.clone()) {
                out.push(MetricCatalogEntry {
                    key: m.key.clone(),
                    title: m.title.clone(),
                    kind: m.display_kind.clone(),
                    language: m.language.clone(),
                    scope: m.scope.clone(),
                    enabled: m.enabled,
                    target: m.target,
                    // A spec has no trigger of its own — its facts arrive on the
                    // producing gauge's cadence.
                    trigger: "auto".to_string(),
                    toggleable: true,
                    category: m.category.clone().or_else(|| Some("custom".to_string())),
                });
            }
        }
        // Built-in producer metrics — listed even with zero recorded data, so the
        // registry is complete the moment a project opens (tsk286). Default-ON and
        // now toggleable (tsk31): a disable marker in config turns one off.
        for p in builtin_producer_metrics() {
            if seen.insert(p.key.to_string()) {
                out.push(MetricCatalogEntry {
                    key: p.key.to_string(),
                    title: p.title.to_string(),
                    kind: p.kind.to_string(),
                    language: None,
                    scope: "built-in".to_string(),
                    enabled: config_state(p.key) != Some(false),
                    target: None,
                    trigger: "auto".to_string(),
                    toggleable: true,
                    category: Some(p.category.to_string()),
                });
            }
        }
        // Every other seeded SPEC — installed plugin metrics and anything else
        // in the spec catalog not covered above (T-E2: the legacy definition
        // table is gone). Best-effort: a store read error just yields the set
        // assembled so far.
        if let Some(facts) = self.fact_store.as_ref() {
            if let Ok(specs) = facts.list_specs().await {
                for s in specs {
                    if seen.insert(s.key.clone()) {
                        out.push(MetricCatalogEntry {
                            key: s.key.clone(),
                            title: s.title.clone(),
                            kind: s.display_kind.clone(),
                            language: s.language.clone(),
                            scope: s.scope.clone(),
                            enabled: config_state(&s.key) != Some(false),
                            target: s.target,
                            // No config trigger for a producer-seeded metric; it
                            // runs on its producer's own cadence.
                            trigger: "auto".to_string(),
                            toggleable: true,
                            category: s.category.clone(),
                        });
                    }
                }
            }
        }
        out.sort_by(|a, b| a.key.cmp(&b.key));
        out
    }

    /// Whether `key` is a default-ON metric (a producer or plugin-seeded spec) —
    /// active unless a `enabled: false` marker disables it. Default-OFF metrics
    /// (built-in code gauges + global `metrics:` definitions) instead activate by
    /// the presence of a `use:` entry. Drives the config edit shape in
    /// [`Self::apply_metric_enabled`].
    fn is_default_on(&self, key: &str) -> bool {
        let is_builtin_gauge = builtin_metrics().iter().any(|m| m.key == key);
        let is_global =
            self.with_global_catalog(|g| g.metrics.iter().any(|e| e.key.as_deref() == Some(key)));
        !is_builtin_gauge && !is_global
    }

    /// Apply one enable/disable to a `metrics:` list in place (no I/O) — the
    /// shared core of [`Self::set_metric_enabled`] and the batch variant so both
    /// stay consistent (tsk31). Default-OFF metrics toggle by `use:` presence;
    /// default-ON metrics and config `key:` definitions toggle by the `enabled`
    /// marker (never deleting a `key:` definition on disable).
    fn apply_metric_enabled(&self, metrics: &mut Vec<MetricEntry>, key: &str, enabled: bool) {
        let pos = metrics
            .iter()
            .position(|e| e.use_key.as_deref() == Some(key) || e.key.as_deref() == Some(key));
        let is_key_def = pos.is_some_and(|i| metrics[i].key.as_deref() == Some(key));
        let default_on = self.is_default_on(key);
        if enabled {
            match pos {
                Some(i) => {
                    metrics[i].enabled = None; // clear any disable marker
                                               // A default-ON metric needs no config entry when active — drop
                                               // a now-bare `use:` marker so config stays clean and
                                               // `resolve_metrics` doesn't warn on the unknown key.
                    let e = &metrics[i];
                    let bare = e.key.is_none()
                        && e.title.is_none()
                        && e.target.is_none()
                        && e.warn_at.is_none()
                        && e.fail_at.is_none();
                    if default_on && !is_key_def && bare {
                        metrics.remove(i);
                    }
                }
                None if !default_on => metrics.push(MetricEntry {
                    use_key: Some(key.to_string()),
                    ..Default::default()
                }),
                None => {} // default-ON with no entry: already active.
            }
        } else {
            match pos {
                // A config `key:` definition — keep it, just flag off (never delete
                // the user's metric).
                Some(i) if is_key_def => metrics[i].enabled = Some(false),
                // Producer/plugin (default-ON): set a disable marker on the entry…
                Some(i) if default_on => metrics[i].enabled = Some(false),
                // …or write a fresh one when there's no entry yet.
                None if default_on => metrics.push(MetricEntry {
                    use_key: Some(key.to_string()),
                    enabled: Some(false),
                    ..Default::default()
                }),
                // Default-OFF (gauge/global): drop the `use:` entry (absence = off),
                // keeping it as a marker only if it carries threshold overrides.
                Some(i) => {
                    let e = &metrics[i];
                    if e.target.is_some() || e.warn_at.is_some() || e.fail_at.is_some() {
                        metrics[i].enabled = Some(false);
                    } else {
                        metrics.remove(i);
                    }
                }
                // Default-OFF with no entry → already off.
                None => {}
            }
        }
    }

    /// Enable or disable a metric in this project's `.oxplow/project.yaml`, then
    /// reseed (the Catalog toggle, tsk219/tsk31). Persists the config + emits
    /// `ConfigChanged`; `seed_catalog` reconciles the `metric_spec` table so a
    /// disabled metric is pruned (hidden + collection stops).
    pub async fn set_metric_enabled(&self, key: &str, enabled: bool) -> Result<(), String> {
        {
            let mut cfg = self
                .config
                .write()
                .map_err(|_| "config lock poisoned".to_string())?;
            self.apply_metric_enabled(&mut cfg.metrics, key, enabled);
            oxplow_config::write_project_config(&self.project_dir, &cfg)
                .map_err(|e| e.to_string())?;
        }
        self.invalidate_global_catalog();
        self.events.emit(OxplowEvent::ConfigChanged);
        self.seed_catalog().await;
        Ok(())
    }

    /// Enable or disable **many** metrics in one config write + one reseed — the
    /// per-section "Enable all / Disable all" action (tsk32). Applies each key to
    /// the same in-memory `metrics:` list under a single lock, then persists once.
    pub async fn set_metrics_enabled(&self, keys: &[String], enabled: bool) -> Result<(), String> {
        {
            let mut cfg = self
                .config
                .write()
                .map_err(|_| "config lock poisoned".to_string())?;
            for key in keys {
                self.apply_metric_enabled(&mut cfg.metrics, key, enabled);
            }
            oxplow_config::write_project_config(&self.project_dir, &cfg)
                .map_err(|e| e.to_string())?;
        }
        self.invalidate_global_catalog();
        self.events.emit(OxplowEvent::ConfigChanged);
        self.seed_catalog().await;
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
        self.invalidate_global_catalog();
        self.events.emit(OxplowEvent::ConfigChanged);
        self.seed_catalog().await;
        Ok(())
    }

    /// Scaffold a new gauge-backed metric (epic tsk12, E): write a starter
    /// Starlark gauge script plus the **trio** that wires it up — a `measures:`
    /// entry (`<key>.count`, the fact type the gauge emits), a `gauges:` entry
    /// (`<key>`, the producer) and a `metrics:` spec (`<key>`, a `sum` over that
    /// measure). Then reseed. Returns the path to the script stub.
    ///
    /// `scope`: `project` (default) writes the script under `oxplow/gauges/` and
    /// the three entries into `.oxplow/project.yaml`, returning the
    /// **project-relative** script path. `global` writes the script + three
    /// manifests under `<global_config_dir>/{gauges,measures,metrics}/` (shared
    /// across the user's projects) plus a project `use:` so the metric charts
    /// here, returning the **absolute** script path.
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
        let slug = slugify(key);
        let measure_key = format!("{key}.count");
        let title = title.filter(|t| !t.is_empty());
        let script = starter_gauge_script(key, &measure_key, &glob, language.as_deref());

        // The `<key>.count` measure the gauge emits (per-file counts). A scaffolded
        // gauge is snapshot-triggered and emits per-FILE facts over a delta, so it
        // is `per-path` by construction (tsk41) — otherwise its metric would read as
        // "only the files in the last commit". Scaffolding it correctly by default
        // is what stops the original bug from being re-introduced by every new gauge.
        let measure = MeasureEntry {
            key: Some(measure_key.clone()),
            title: Some(format!("{key} (per-file count)")),
            unit: Some("count".to_string()),
            subject_kind: Some("file".to_string()),
            temporal_semantics: Some("semi-additive".to_string()),
            capture_scope: Some("per-path".to_string()),
            component_role: None,
            description: None,
        };
        // The gauge (producer) — emits `<key>.count` facts on every snapshot.
        let gauge = GaugeEntry {
            key: Some(key.to_string()),
            title: title.clone(),
            trigger: Some("on-snapshot".to_string()),
            emits: vec![measure_key.clone()],
            compute: Some(GaugeComputeConfig {
                runtime: "starlark".to_string(),
                input: None,
                entry_file: Some(if global {
                    format!("{slug}.star")
                } else {
                    format!("oxplow/gauges/{slug}.star")
                }),
                args: vec![],
                report: None,
            }),
        };
        // The metric (spec) — a `sum` over the measure's facts.
        let metric = MetricEntry {
            key: Some(key.to_string()),
            title,
            source_measure: Some(measure_key.clone()),
            aggregation: Some("sum".to_string()),
            display_kind: Some("gauge".to_string()),
            language,
            ..Default::default()
        };

        let returned_path = if global {
            let gdir = self
                .effective_global_dir()
                .ok_or_else(|| "no global config dir available on this platform".to_string())?;
            let already = load_global_metric_entries(&gdir)
                .iter()
                .any(|e| e.key.as_deref() == Some(key))
                || load_global_gauge_entries(&gdir)
                    .iter()
                    .any(|e| e.key.as_deref() == Some(key));
            if already {
                return Err(format!("global metric `{key}` already exists"));
            }
            let gauges_dir = gdir.join("gauges");
            std::fs::create_dir_all(&gauges_dir).map_err(|e| e.to_string())?;
            let script_abs = gauges_dir.join(format!("{slug}.star"));
            if !script_abs.exists() {
                std::fs::write(&script_abs, script).map_err(|e| e.to_string())?;
            }
            oxplow_config::write_global_measures_file(
                &gdir.join("measures").join(format!("{slug}.yaml")),
                &[measure],
            )
            .map_err(|e| e.to_string())?;
            oxplow_config::write_global_gauges_file(
                &gauges_dir.join(format!("{slug}.yaml")),
                &[gauge],
            )
            .map_err(|e| e.to_string())?;
            oxplow_config::write_global_metrics_file(
                &gdir.join("metrics").join(format!("{slug}.yaml")),
                &[metric],
            )
            .map_err(|e| e.to_string())?;
            // The global metric is library content — enable it here with a project
            // `use:` so it charts in this project. The global gauge + measure are
            // active automatically (loaded from the global dir at seed time).
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
            let script_rel = format!("oxplow/gauges/{slug}.star");
            {
                let mut cfg = self
                    .config
                    .write()
                    .map_err(|_| "config lock poisoned".to_string())?;
                if cfg.metrics.iter().any(|e| e.key.as_deref() == Some(key))
                    || cfg.gauges.iter().any(|e| e.key.as_deref() == Some(key))
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
                cfg.measures.push(measure);
                cfg.gauges.push(gauge);
                cfg.metrics.push(metric);
                oxplow_config::write_project_config(&self.project_dir, &cfg)
                    .map_err(|e| e.to_string())?;
            }
            script_rel
        };

        self.invalidate_global_catalog();
        self.events.emit(OxplowEvent::ConfigChanged);
        self.seed_catalog().await;
        Ok(returned_path)
    }

    /// True when at least one on-snapshot gauge still needs a full-tree
    /// baseline in this stream — a fresh project, a newly added gauge, or a
    /// gauge whose script changed since its last baseline.
    ///
    /// A `per-path` measure's fold needs each gauge to have restated the whole
    /// tree at least once; delta captures alone would let the metric creep up
    /// from 0 over months instead of reporting the repo (tsk41). The baseline
    /// is a `scan_kind = 'full'` capture over the RECONSTRUCTED tree of an
    /// ordinary snapshot (tsk71) — the on-snapshot sweep drains
    /// [`Self::gauges_needing_baseline`] on the next snapshot that lands.
    pub async fn needs_tree_baseline(&self, stream_id: i64) -> bool {
        !self.gauges_needing_baseline(stream_id).await.is_empty()
    }

    /// Enabled on-snapshot gauges that have not been baselined at their CURRENT logic —
    /// i.e. that need a full-tree run before their metric is trustworthy.
    ///
    /// The question is per GAUGE, not per measure (tsk49): `oxplow.ast_hit` is one
    /// measure shared by 10 idiom gauges, so "does the measure have facts" tells you
    /// nothing about one gauge — a delta-only gauge looks done because a sibling filled
    /// the measure. A gauge is un-baselined when it has no completed
    /// `scan_kind = 'full'` capture at its current fingerprint (tsk71): that one
    /// check covers both "never scanned the whole tree" and "script changed since
    /// the last baseline" (the old `gauge_is_stale` criterion — a full capture at
    /// stale logic carries the old fingerprint and doesn't match).
    ///
    /// This set is the pending-baseline QUEUE: the on-snapshot sweep drains it by
    /// running these gauges `full` over the next snapshot that lands, so a newly
    /// added or edited gauge baselines on the next ordinary snapshot — no
    /// fabricated full-tree snapshot (which used to pollute effort attribution).
    pub async fn gauges_needing_baseline(&self, stream_id: i64) -> Vec<String> {
        let Some(facts) = self.fact_store.as_ref() else {
            return Vec::new();
        };
        // No snapshot yet (fresh project) — nothing to anchor a baseline on.
        let has_snapshot = matches!(
            self.snapshot_store
                .latest_snapshot_id_for_stream(StreamId::new(stream_id))
                .await,
            Ok(Some(_))
        );
        if !has_snapshot {
            return Vec::new();
        }
        let mut out = Vec::new();
        for gauge in self.resolved_gauges() {
            if gauge.trigger != "on-snapshot" {
                continue;
            }
            // Fingerprint-scoped when the script is hashable; any-version
            // otherwise (an unfingerprintable gauge can't detect staleness,
            // so one full capture ever is the best we can require).
            let fp = gauge_fingerprint(&gauge, &self.script_base_dir(&gauge));
            let baselined = facts
                .has_full_capture(&gauge.key, stream_id, fp.as_deref())
                .await
                .unwrap_or(true); // read failure: don't stampede a re-baseline
            if !baselined {
                out.push(gauge.key.clone());
            }
        }
        out
    }

    /// Whether ONE gauge's facts were computed by logic that has since changed — its
    /// current fingerprint vs the one recorded on its latest capture.
    ///
    /// `false` when it has never run (the empty-fold check covers that) or when its
    /// script can't be fingerprinted (better to skip than to re-baseline the whole
    /// tree on every boot over an unreadable file).
    pub async fn gauge_is_stale(&self, gauge: &ResolvedGauge, stream_id: i64) -> bool {
        let Some(facts) = self.fact_store.as_ref() else {
            return false;
        };
        let Some(current) = gauge_fingerprint(gauge, &self.script_base_dir(gauge)) else {
            return false;
        };
        let recorded = match facts.latest_producer_version(&gauge.key, stream_id).await {
            Ok(None) | Err(_) => return false, // never captured
            Ok(Some(v)) => v,
        };
        let stale = recorded.as_deref() != Some(current.as_str());
        if stale {
            tracing::info!(
                gauge = %gauge.key,
                "gauge logic changed since its last capture — re-baseline due",
            );
        }
        stale
    }

    /// Scaffold a custom **measure** (a new fact TYPE) — epic tsk12, E. Appends a
    /// `measures:` entry to `.oxplow/project.yaml` (project scope, default) or
    /// writes a shareable `<global>/measures/<slug>.yaml` (global scope), then
    /// reseeds the catalog. Returns the created measure key. A global measure is
    /// active in every project automatically (`seed_catalog` loads global +
    /// project), so — unlike a metric — no project `use:` opt-in is written.
    ///
    /// The measure's fields ride in a [`MeasureEntry`] — including
    /// `captureScope` (`complete` (default) | `per-path`), which a
    /// snapshot-triggered tree gauge's measure must set to `per-path` so its metric
    /// reads the whole repo rather than just the last commit's files (tsk41).
    pub async fn scaffold_measure(
        &self,
        entry: MeasureEntry,
        scope: Option<String>,
    ) -> Result<String, String> {
        let key_owned = entry.key.as_deref().unwrap_or_default().trim().to_string();
        let key: &str = &key_owned;
        if key.is_empty() || !key.contains('.') {
            return Err("key must be namespaced, e.g. acme.api_latency".to_string());
        }
        if key.starts_with("oxplow.") {
            return Err("`oxplow.` is reserved for built-in measures".to_string());
        }
        let blank = |v: Option<String>| v.filter(|s| !s.is_empty());
        let entry = MeasureEntry {
            key: Some(key.to_string()),
            title: blank(entry.title),
            unit: blank(entry.unit),
            subject_kind: blank(entry.subject_kind),
            temporal_semantics: blank(entry.temporal_semantics),
            capture_scope: blank(entry.capture_scope),
            component_role: None,
            description: blank(entry.description),
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
        self.invalidate_global_catalog();
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
        self.invalidate_global_catalog();
        self.events.emit(OxplowEvent::ConfigChanged);
        self.seed_catalog().await;
        Ok(key.to_string())
    }

    /// Event loop: seed once, then reseed on `ConfigChanged` and run on-snapshot
    /// gauges when a snapshot batch lands. Spawned at boot (see `boot.rs`).
    pub async fn run(self, mut rx: tokio::sync::broadcast::Receiver<OxplowEvent>) {
        self.seed_catalog().await;
        loop {
            match rx.recv().await {
                Ok(OxplowEvent::ConfigChanged) => {
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
    /// `pub(crate)` so [`crate::Services::rebuild_metric_baseline`] can drive it
    /// directly (and thus test the boot path end to end, tsk50).
    ///
    /// Two-phase (tsk71): gauges already baselined run a `delta` scan over the
    /// snapshot's own file rows (the cheap incremental rescan); gauges in the
    /// pending-baseline queue ([`Self::gauges_needing_baseline`]) run a `full`
    /// scan over the RECONSTRUCTED tree as-of this snapshot and record
    /// `scan_kind = 'full'` captures anchored to it. So a baseline needs no
    /// fabricated full-tree snapshot — it piggybacks on whatever ordinary
    /// snapshot lands next.
    pub(crate) async fn run_snapshot_gauges(
        &self,
        stream_id: StreamId,
        snapshot_id: i64,
    ) -> SweepReport {
        self.run_snapshot_gauges_with(stream_id, snapshot_id, false)
            .await
    }

    /// [`Self::run_snapshot_gauges`] with a `force_full` override: treat EVERY
    /// on-snapshot gauge as needing a baseline (the `rebuild_metrics(force)`
    /// escape hatch). The per-snapshot idempotency guard still applies, so a
    /// repeated force over the same unchanged snapshot doesn't re-scan.
    pub(crate) async fn run_snapshot_gauges_with(
        &self,
        stream_id: StreamId,
        snapshot_id: i64,
        force_full: bool,
    ) -> SweepReport {
        let gauges: Vec<ResolvedGauge> = self
            .resolved_gauges()
            .into_iter()
            .filter(|g| g.trigger == "on-snapshot")
            .collect();
        if gauges.is_empty() {
            return SweepReport::default();
        }
        let needing: std::collections::HashSet<String> = if force_full {
            gauges.iter().map(|g| g.key.clone()).collect()
        } else {
            self.gauges_needing_baseline(stream_id.value())
                .await
                .into_iter()
                .collect()
        };
        let (full_gauges, delta_gauges): (Vec<ResolvedGauge>, Vec<ResolvedGauge>) =
            gauges.into_iter().partition(|g| needing.contains(&g.key));

        let mut report = SweepReport::default();
        if !delta_gauges.is_empty() {
            // The snapshot's own rows — the incremental rescan corpus.
            let files = Arc::new(self.build_file_map(snapshot_id).await);
            let ctx = self
                .snapshot_context(stream_id.value(), None, "on-snapshot", snapshot_id)
                .await;
            let r = self
                .run_gauge_sweep(&delta_gauges, &ctx, files, stream_id.value())
                .await;
            report.ran += r.ran;
            report.failed.extend(r.failed);
        }
        if !full_gauges.is_empty() {
            // The reconstructed whole tree as-of this snapshot — the baseline
            // corpus. Built only when something actually needs baselining.
            let files = Arc::new(self.build_full_file_map(snapshot_id).await);
            let mut ctx = self
                .snapshot_context(stream_id.value(), None, "on-snapshot", snapshot_id)
                .await;
            ctx.scan_kind = "full";
            let r = self
                .run_gauge_sweep(&full_gauges, &ctx, files, stream_id.value())
                .await;
            let full_ok = r.failed.is_empty();
            report.ran += r.ran;
            report.failed.extend(r.failed);
            // A fresh baseline makes every older effort-less tree capture dead
            // weight (tsk75 — their facts were ~69% of the table and every
            // full-history read paid for them). Prune only on a clean sweep:
            // a failed gauge wrote no baseline, so its history must survive.
            if full_ok {
                if let Some(facts) = self.fact_store.as_ref() {
                    match facts.prune_dominated_tree_captures(stream_id.value()).await {
                        Ok(n) if n > 0 => {
                            tracing::info!(
                                pruned = n,
                                "metrics: dropped baseline-dominated tree captures"
                            );
                        }
                        Ok(_) => {}
                        Err(e) => {
                            tracing::warn!(error = %e, "metrics: dominated-capture prune failed");
                        }
                    }
                }
            }
        }
        report
    }

    /// Run `gauges` over one file map, reporting progress when it's a whole-tree
    /// sweep. Split out from [`Self::run_snapshot_gauges`] so the tracking is
    /// exercisable without standing up real snapshot blobs.
    async fn run_gauge_sweep(
        &self,
        gauges: &[ResolvedGauge],
        ctx: &GaugeRunContext,
        files: Arc<HashMap<String, String>>,
        stream_val: i64,
    ) -> SweepReport {
        // Idempotency: skip a gauge that already has a `done` capture for THIS
        // snapshot at its current fingerprint (tsk50). Otherwise a re-delivered
        // snapshot event — or the direct baseline run PLUS the event loop reacting to
        // the same snapshot — would tree-sitter-parse the whole tree twice (minutes of
        // CPU). The manual `run_metric` path doesn't come through here, so an explicit
        // "run now" still runs.
        let mut to_run: Vec<&ResolvedGauge> = Vec::new();
        for g in gauges {
            let already = match (ctx.snapshot_id, self.fact_store.as_ref()) {
                (Some(snap), Some(facts)) => {
                    let fp = gauge_fingerprint(g, &self.script_base_dir(g));
                    facts
                        .gauge_done_for_snapshot(&g.key, snap, fp.as_deref(), ctx.scan_kind)
                        .await
                        .unwrap_or(false)
                }
                _ => false,
            };
            if !already {
                to_run.push(g);
            }
        }
        if to_run.is_empty() {
            return SweepReport::default();
        }

        // A WHOLE-TREE sweep (the baseline) tree-sitter-parses every file for every
        // gauge — minutes of CPU. Track it as a background task so the user can see
        // what oxplow is doing and why a core is pinned (tsk48). An ordinary delta
        // (a handful of changed files) finishes in milliseconds and would only be
        // noise, so it stays untracked. `run()` processes snapshot events serially,
        // so two sweeps can never overlap.
        let tracked = (files.len() >= TREE_SWEEP_FILE_THRESHOLD)
            .then_some(self.background_tasks.as_ref())
            .flatten();
        let task = tracked.map(|bts| {
            bts.start(crate::background_task::StartInput {
                kind: crate::background_task::BackgroundTaskKind::Metrics,
                label: format!("Computing code metrics ({} files)", files.len()),
                progress: Some(0.0),
                ..Default::default()
            })
        });

        let mut failed: Vec<String> = Vec::new();
        for (i, g) in to_run.iter().enumerate() {
            if let (Some(bts), Some(t)) = (tracked, task.as_ref()) {
                bts.update(
                    &t.id,
                    crate::background_task::UpdateInput {
                        label: Some(format!(
                            "Computing code metrics ({}/{}) — {}",
                            i + 1,
                            to_run.len(),
                            g.key
                        )),
                        progress: Some(Some(i as f64 / to_run.len() as f64)),
                        ..Default::default()
                    },
                );
            }
            // `run_one_gauge` returns 0 both for "found nothing" and "failed", so ask
            // the substrate which it was rather than guessing.
            self.run_one_gauge(g, ctx, files.clone()).await;
            if self.last_run_failed(&g.key, stream_val).await {
                failed.push(g.key.clone());
            }
        }

        if let (Some(bts), Some(t)) = (tracked, task.as_ref()) {
            if failed.is_empty() {
                bts.complete(
                    &t.id,
                    Some(serde_json::json!({ "gauges": to_run.len(), "files": files.len() })),
                );
            } else {
                // Do NOT silently succeed. A failed gauge leaves its metric reading
                // stale or empty, and that going unnoticed is exactly the bug (tsk47).
                bts.fail(
                    &t.id,
                    format!(
                        "{} of {} gauges failed: {}",
                        failed.len(),
                        to_run.len(),
                        failed.join(", ")
                    ),
                    None,
                );
            }
        }
        SweepReport {
            ran: to_run.len(),
            failed,
        }
    }

    /// Whether this producer's LATEST capture is a failure — i.e. the gauge we just
    /// ran errored. `run_one_gauge` can't tell us directly (it returns 0 for both
    /// "found nothing" and "blew up"), but the failure capture (tsk47) can.
    async fn last_run_failed(&self, producer: &str, stream_id: i64) -> bool {
        let Some(facts) = self.fact_store.as_ref() else {
            return false;
        };
        facts
            .latest_capture_status(producer, stream_id)
            .await
            .ok()
            .flatten()
            .is_some_and(|s| s == "failed")
    }

    /// Run every enabled `on-effort-complete` gauge when an effort closes. The
    /// file map comes from the effort's end snapshot (the worktree as it stood
    /// at close); samples default their subject to the effort.
    pub async fn run_effort_complete_gauges(&self, thread_id: &ThreadId, effort_id: &EffortId) {
        let gauges: Vec<ResolvedGauge> = self
            .resolved_gauges()
            .into_iter()
            .filter(|g| g.trigger == "on-effort-complete")
            .collect();
        if gauges.is_empty() {
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
        let mut ctx = self
            .snapshot_context(
                stream_val,
                Some(thread_id.value()),
                "on-effort-complete",
                snapshot_id.unwrap_or(0),
            )
            .await;
        // This trigger KNOWS the producing effort — stamp it so the capture is
        // attributable via `captures_for_effort` (tsk43; the pre-arc effort
        // subject default was removed without a replacement).
        ctx.effort_id = Some(effort_id.value());
        for g in &gauges {
            self.run_one_gauge(g, &ctx, files.clone()).await;
        }
    }

    /// Manually run one configured gauge by key, against the stream's latest
    /// snapshot. Returns the number of samples recorded, or an error string.
    /// (The MCP `run_metric` tool, tsk226, calls this.)
    pub async fn run_metric_by_key(
        &self,
        key: &str,
        stream: Option<StreamId>,
    ) -> Result<usize, String> {
        let metric = self
            .resolved_gauges()
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
            .snapshot_context(stream_val, None, "manual", snapshot_id.unwrap_or(0))
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
        self.file_map_from_rows(files).await
    }

    /// Build the RECONSTRUCTED whole-tree file map as-of `snapshot_id` (the
    /// latest row per path ≤ the snapshot, tombstones excluded) — the baseline
    /// corpus (tsk71). Same content pipeline as [`Self::build_file_map`]; only
    /// the listing differs.
    async fn build_full_file_map(&self, snapshot_id: i64) -> HashMap<String, String> {
        let files = self
            .snapshot_store
            .list_tree_files_at(snapshot_id)
            .await
            .unwrap_or_default();
        self.file_map_from_rows(files).await
    }

    /// Read each row's content (blob store or git odb) into a path→text map,
    /// skipping deleted/oversize/binary/over-large files.
    async fn file_map_from_rows(
        &self,
        files: Vec<oxplow_db::FileSnapshot>,
    ) -> HashMap<String, String> {
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
            effort_id: None,
            scan_kind: "delta",
        }
    }

    /// Run one gauge: build its collector, execute under the sandbox with the
    /// file-map host, and record a run + a sample per `MetricReport.sample`.
    /// Best-effort — errors are logged and swallowed. Returns the sample count.
    async fn run_one_gauge(
        &self,
        gauge: &ResolvedGauge,
        ctx: &GaugeRunContext,
        files: Arc<HashMap<String, String>>,
    ) -> usize {
        // Built-in gauges run from their embedded script (no project-disk file);
        // global/project gauges build from their `compute.entryFile`.
        let collector = if gauge.scope == "built-in" {
            match builtin_collector(&gauge.key) {
                Some(c) => c,
                None => {
                    tracing::warn!(key = %gauge.key, "gauge: unknown built-in key");
                    return 0;
                }
            }
        } else {
            // A global gauge's script lives under the global config dir, not the
            // project; project gauges resolve against the project dir.
            let base = self.script_base_dir(gauge);
            match compute_to_collector(gauge, &base) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(key = %gauge.key, error = %e, "gauge: bad compute");
                    return 0;
                }
            }
        };
        let source = gauge_source(gauge, &collector);
        // The report-derived content (if any); tree-derived gauges ignore it.
        let content = match &gauge.compute.report {
            Some(rel) => {
                std::fs::read_to_string(self.script_base_dir(gauge).join(rel)).unwrap_or_default()
            }
            None => String::new(),
        };
        // A tree gauge scans the WHOLE tree (hundreds of files, tree-sitter each) —
        // the 5s default was sized for report parsers over a single small file and is
        // nowhere near enough. Under it, the broad-query gauges silently timed out on
        // every full-tree run: `oxplow.ts.console_calls` and `oxplow.ts.ts_ignore` had
        // produced ZERO facts since the project was indexed, while the repo held 137
        // console calls (tsk47). Gauges run detached under `spawn_blocking`, so a
        // generous ceiling costs nothing and still catches a runaway script.
        let collector = collector.with_budget(SandboxBudget::with_timeout(GAUGE_TIMEOUT));
        let host = GaugeHost::from_shared(files);
        let started = std::time::Instant::now();
        let report =
            match tokio::task::spawn_blocking(move || collector.run_gauge(&content, host)).await {
                Ok(Ok(out)) => out,
                Ok(Err(e)) => {
                    // NOT a silent warn. A gauge that fails leaves its metric reading
                    // stale-or-empty forever, and that is exactly how two built-in
                    // metrics went unnoticed for weeks. Record the failure durably so
                    // it can be seen and reasoned about.
                    self.record_gauge_failure(gauge, ctx, &source, &e.to_string())
                        .await;
                    tracing::error!(
                        key = %gauge.key,
                        error = %e,
                        elapsed_ms = started.elapsed().as_millis() as u64,
                        "gauge: compute FAILED — its metric will read stale or empty",
                    );
                    return 0;
                }
                Err(e) => {
                    tracing::error!(key = %gauge.key, error = %e, "gauge: join failed");
                    return 0;
                }
            };
        tracing::debug!(
            key = %gauge.key,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "gauge: complete",
        );
        let gauge_facts = match report {
            oxplow_collect_plugin::CollectorOutput::Gauge(r) => r.facts,
            _ => return 0,
        };

        // Inverted substrate (epic tsk12): the gauge's durable atomic `facts` are
        // the ONLY output recorded — the metric reads aggregate them via the
        // engine (the read flip, T-C3). Any legacy `samples`/`findings` a script
        // still returns are ignored. Best-effort; never fails the gauge run.
        self.record_gauge_facts(gauge, ctx, &source, &gauge_facts)
            .await
    }

    /// Record a FAILED gauge run as a `status = 'failed'` capture (tsk47).
    ///
    /// Two reasons this must be durable rather than a log line:
    /// 1. **Visibility.** A gauge that fails leaves its metric reading stale or empty
    ///    *forever*, and nothing said so — `oxplow.ts.console_calls` read empty for
    ///    weeks against a repo with 137 console calls. A metric that is obviously
    ///    broken is far better than one that is quietly wrong.
    /// 2. **It stops the boot loop.** The capture carries the gauge's fingerprint, so
    ///    `gauge_is_stale` sees the current logic *was* attempted and doesn't demand a
    ///    fresh full-tree baseline on every single boot.
    ///
    /// It carries NO facts, and the read folds skip non-`done` captures — critical,
    /// because an empty capture over a *full-tree* snapshot restates every path, and
    /// would otherwise supersede everything and zero the metric.
    async fn record_gauge_failure(
        &self,
        gauge: &ResolvedGauge,
        ctx: &GaugeRunContext,
        source: &str,
        error: &str,
    ) {
        let Some(facts) = self.fact_store.as_ref() else {
            return;
        };
        let mut capture = oxplow_db::NewMetricCapture::done(
            ctx.stream_val,
            gauge.key.clone(),
            source.to_string(),
        );
        capture.status = "failed".into();
        capture.error = Some(error.to_string());
        capture.thread_id = ctx.thread_id;
        capture.effort_id = ctx.effort_id;
        capture.scope = Some(gauge.scope.clone());
        capture.trigger = Some(ctx.trigger.into());
        capture.snapshot_id = ctx.snapshot_id;
        capture.closest_git_version = ctx.closest_git_version.clone();
        capture.git_version_exact = ctx.git_version_exact;
        capture.branch = ctx.branch.clone();
        capture.producer_version = gauge_fingerprint(gauge, &self.script_base_dir(gauge));
        capture.scan_kind = ctx.scan_kind.into();
        if let Err(e) = facts.record_facts(capture, Vec::new()).await {
            tracing::warn!(key = %gauge.key, error = %e, "gauge: failure record write failed");
        }
    }

    /// Persist a gauge's per-item `facts` (epic tsk12) as `fact` rows under one
    /// `metric_capture`, resolving each fact's measure key to a defined measure.
    /// Enforces **declare-to-collect** (decision #4): a fact is dropped (surfaced
    /// via `tracing::warn!`, never silently written) if its measure is undefined
    /// in the catalog OR not in the gauge's own `emits` allow-list (a config gauge
    /// may only emit the measures it declares). A built-in gauge has an empty
    /// `emits` — the catalog check alone governs it.
    ///
    /// A ZERO-fact run still writes its (empty) capture — "this scan ran and
    /// found nothing" is the record that lets a count metric drop back to zero
    /// after the last offender is fixed; the engine zero-fills the series from
    /// the producer's captures (tsk44). Returns the number of facts recorded.
    /// Emits `MetricSamplesChanged` when it writes. Best-effort.
    async fn record_gauge_facts(
        &self,
        gauge: &ResolvedGauge,
        ctx: &GaugeRunContext,
        source: &str,
        gauge_facts: &[oxplow_collect_plugin::GaugeFact],
    ) -> usize {
        let Some(facts) = self.fact_store.as_ref() else {
            return 0;
        };
        // Resolve the measure catalog once (one query), then map each fact's key.
        let by_key: HashMap<String, i64> = match facts.list_measures().await {
            Ok(ms) => ms.into_iter().map(|m| (m.key, m.id)).collect(),
            Err(e) => {
                tracing::warn!(key = %gauge.key, error = %e, "gauge facts: measure catalog read failed");
                return 0;
            }
        };
        let mut rows = Vec::new();
        for gf in gauge_facts {
            // A non-finite measurement isn't meaningful — drop it (mirrors the
            // sample guard) rather than poison the fact stream.
            if !gf.value.is_finite() {
                continue;
            }
            // The gauge's own contract: a config gauge may only emit measures it
            // declared in `emits` (built-ins declare none → unrestricted).
            if !gauge.emits.is_empty() && !gauge.emits.iter().any(|m| m == &gf.measure) {
                tracing::warn!(
                    key = %gauge.key, measure = %gf.measure,
                    "gauge facts: measure not in the gauge's `emits` — fact dropped"
                );
                continue;
            }
            let Some(&measure_id) = by_key.get(gf.measure.as_str()) else {
                // Declare-to-collect: a gauge may only emit DEFINED measures.
                tracing::warn!(
                    key = %gauge.key, measure = %gf.measure,
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
                // The reported rule/idiom — the engine reads this column as the
                // `oxplow.rule` dimension, so a spec can `dim_eq` on it.
                rule: gf.rule.clone(),
                // Ratio components — carried so a `ratio` spec re-derives Σnum/Σden.
                numerator: gf.num,
                denominator: gf.den,
                dims_json: gf.dims.as_ref().and_then(|d| serde_json::to_string(d).ok()),
                ..oxplow_db::NewFact::new(measure_id, gf.value)
            });
        }
        // No `rows.is_empty()` bail: the empty capture IS the zero record.
        let count = rows.len();
        let capture = oxplow_db::NewMetricCapture {
            thread_id: ctx.thread_id,
            effort_id: ctx.effort_id,
            scope: Some(gauge.scope.clone()),
            trigger: Some(ctx.trigger.into()),
            basis_ref: ctx.closest_git_version.clone(),
            snapshot_id: ctx.snapshot_id,
            closest_git_version: ctx.closest_git_version.clone(),
            git_version_exact: ctx.git_version_exact,
            branch: ctx.branch.clone(),
            // Record WHICH LOGIC produced these facts, so a later script change is
            // detectable and can re-baseline instead of silently no-opping (tsk45).
            producer_version: gauge_fingerprint(gauge, &self.script_base_dir(gauge)),
            scan_kind: ctx.scan_kind.into(),
            ..oxplow_db::NewMetricCapture::done(
                ctx.stream_val,
                gauge.key.clone(),
                source.to_string(),
            )
        };
        if let Err(e) = facts.record_facts(capture, rows).await {
            tracing::warn!(key = %gauge.key, error = %e, "gauge facts: record failed");
            return 0;
        }
        self.events.emit(OxplowEvent::MetricSamplesChanged {
            stream_id: StreamId::new(ctx.stream_val),
        });
        count
    }
}

/// Map a resolved `ResolvedSpec` to a `metric_spec` write (for config-declared
/// metrics). A formula metric has no `source_measure`.
fn spec_to_new_spec(s: &ResolvedSpec) -> NewMetricSpec {
    NewMetricSpec {
        key: s.key.clone(),
        title: s.title.clone(),
        unit: s.unit.clone(),
        source_measure: s.source_measure.clone(),
        aggregation: s.aggregation.clone(),
        filter_json: s.filter.as_ref().map(filter_to_json),
        formula: s.formula.as_ref().map(formula_to_json),
        sliceable_dims_json: (!s.sliceable_dims.is_empty())
            .then(|| serde_json::to_string(&s.sliceable_dims).unwrap_or_else(|_| "[]".into())),
        direction: s.direction.clone(),
        target: s.target,
        warn_at: s.warn_at,
        fail_at: s.fail_at,
        description: s.description.clone(),
        category: s.category.clone(),
        language: s.language.clone(),
        scope: s.scope.clone(),
        display_kind: s.display_kind.clone(),
    }
}

/// Serialize a config `FilterConfig` to the engine's `filter_json` shape
/// (`FactFilter`: `min_value` / `severity` / `dim_eq`).
fn filter_to_json(f: &oxplow_config::FilterConfig) -> String {
    let mut m = serde_json::Map::new();
    if let Some(v) = f.min_value {
        m.insert("min_value".into(), serde_json::json!(v));
    }
    if let Some(s) = &f.severity {
        m.insert("severity".into(), serde_json::json!(s));
    }
    if let Some(pair) = &f.dim_eq {
        if pair.len() == 2 {
            m.insert("dim_eq".into(), serde_json::json!([pair[0], pair[1]]));
        }
    }
    serde_json::Value::Object(m).to_string()
}

/// Serialize a config `FormulaConfig` to the engine's `formula` shape
/// (`{op, left, right}`).
fn formula_to_json(f: &oxplow_config::FormulaConfig) -> String {
    serde_json::json!({ "op": f.op, "left": f.left, "right": f.right }).to_string()
}

/// The bundled built-in metric catalog as spec-shaped `MetricEntry`s, so the
/// three-scope resolver knows them (a project `use:`s one to activate it). The
/// structural spec fields (`source_measure`/`aggregation`/`filter`) are joined in
/// from the hand-written built-in specs by key; the surface fields come from
/// `builtin_metrics()`.
fn builtin_spec_entries() -> Vec<MetricEntry> {
    let specs: HashMap<String, NewMetricSpec> = builtin_metric_specs()
        .into_iter()
        .chain(builtin_ast_specs())
        .map(|s| (s.key.clone(), s))
        .collect();
    builtin_metrics()
        .iter()
        .map(|m| {
            let spec = specs.get(m.key);
            MetricEntry {
                key: Some(m.key.to_string()),
                title: Some(m.title.to_string()),
                source_measure: spec.and_then(|s| s.source_measure.clone()),
                aggregation: spec.map(|s| s.aggregation.clone()),
                filter: spec.and_then(|s| filter_from_json(s.filter_json.as_deref())),
                unit: Some(m.unit.to_string()),
                direction: Some(m.direction.to_string()),
                display_kind: Some(m.kind.to_string()),
                category: spec.and_then(|s| s.category.clone()),
                // Empty language = a language-agnostic metric (the unified code
                // metrics) — no single language (NULL on the definition).
                language: (!m.language.is_empty()).then(|| m.language.to_string()),
                description: Some(m.description.to_string()),
                sliceable_dims: m.dimensions.iter().map(|d| d.to_string()).collect(),
                target: m.target,
                ..Default::default()
            }
        })
        .collect()
}

/// Parse a built-in spec's `filter_json` back into a config `FilterConfig` (so a
/// `use:` re-seed reconstructs the same predicate).
fn filter_from_json(json: Option<&str>) -> Option<oxplow_config::FilterConfig> {
    let engine_filter = crate::metric_engine::FactFilter::from_json(json?).ok()?;
    Some(oxplow_config::FilterConfig {
        min_value: engine_filter.min_value,
        severity: engine_filter.severity,
        dim_eq: engine_filter.dim_eq.map(|(k, v)| vec![k, v]),
    })
}

/// A built-in gauge as a `ResolvedGauge` — `run_one_gauge` builds its collector
/// from the embedded script (never a project-disk file), so the compute is a
/// default sentinel and `emits` is empty (built-ins are catalog-governed, not
/// `emits`-restricted).
fn builtin_gauge(key: &str, trigger: &str) -> ResolvedGauge {
    ResolvedGauge {
        key: key.to_string(),
        title: key.to_string(),
        trigger: trigger.to_string(),
        emits: Vec::new(),
        compute: GaugeComputeConfig::default(),
        scope: "built-in".to_string(),
    }
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

/// Built-in metric SPECS for the per-language idiom gauges (epic tsk12, tsk30) —
/// `oxplow.rust.unsafe_blocks` and friends. Each is a `Sum(oxplow.ast_hit)`
/// filtered to its idiom via `dim_eq(oxplow.rule, <slug>)`; the per-file
/// `oxplow.ast_hit` facts each gauge emits (rule-tagged) sum back to the baked
/// `tree:.` headline — pinned by the equivalence test. The `<slug>` MUST match
/// the `rule` the gauge script emits.
fn builtin_ast_specs() -> Vec<NewMetricSpec> {
    fn ast_spec(key: &str, title: &str, rule: &str, direction: &str) -> NewMetricSpec {
        let mut s = NewMetricSpec::base(key, title, "oxplow.ast_hit", "sum");
        s.unit = Some("count".into());
        s.filter_json = Some(format!("{{\"dim_eq\":[\"oxplow.rule\",\"{rule}\"]}}"));
        s.direction = direction.into();
        s.display_kind = "findings".into();
        s.category = Some("static-quality".into());
        s
    }
    vec![
        ast_spec(
            "oxplow.rust.unsafe_blocks",
            "unsafe blocks",
            "unsafe_block",
            "lower-better",
        ),
        ast_spec(
            "oxplow.rust.unwrap_expect_calls",
            "unwrap / expect calls",
            "unwrap_expect",
            "lower-better",
        ),
        ast_spec(
            "oxplow.rust.panic_macros",
            "panic-family macros",
            "panic_macro",
            "lower-better",
        ),
        ast_spec(
            "oxplow.ts.any_usage",
            "any usage",
            "any_usage",
            "lower-better",
        ),
        ast_spec(
            "oxplow.ts.non_null_assertions",
            "non-null assertions",
            "non_null_assertion",
            "lower-better",
        ),
        ast_spec(
            "oxplow.ts.console_calls",
            "console.* calls",
            "console_call",
            "lower-better",
        ),
        ast_spec(
            "oxplow.ts.ts_ignore",
            "ts-ignore / ts-expect-error",
            "ts_ignore",
            "lower-better",
        ),
        ast_spec("oxplow.clojure.defn_count", "defn count", "defn", "neutral"),
        ast_spec(
            "oxplow.csharp.empty_catch",
            "empty catch blocks",
            "empty_catch",
            "lower-better",
        ),
        ast_spec(
            "oxplow.csharp.blocking_async_calls",
            "blocking async calls (.Result / .Wait())",
            "blocking_async",
            "lower-better",
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

/// A filesystem-safe slug from a namespaced key (non-alphanumerics → `_`), for
/// naming the global scaffold's `<slug>.yaml` / `<slug>.star` files.
fn slugify(key: &str) -> String {
    key.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// The starter Starlark gauge written by [`MetricsService::scaffold_metric`]. A
/// working tree-derived gauge that emits one per-file `<measure>` FACT per
/// matched file (TODO/FIXME count) — the metric spec (`sum` over `<measure>`)
/// charts it. Emits facts only (no baked sample), the clean substrate model
/// (epic tsk12); an `ast_query` example is in a comment for the author.
fn starter_gauge_script(key: &str, measure: &str, glob: &str, language: Option<&str>) -> String {
    let lang = language.unwrap_or("rust");
    format!(
        "# {key} — a tree-derived gauge. Reads the snapshot via files() and (optionally)\n\
         # the AST via ast_query(); deterministic (no I/O) so facts are `observed`.\n\
         #\n\
         # Emits one per-file fact on the `{measure}` measure (count of TODO/FIXME).\n\
         # To count an AST node instead, e.g.:\n\
         #   c = len(ast_query(f[\"text\"], \"{lang}\", \"(identifier) @x\"))\n\
         def transform(input):\n    \
             facts = []\n    \
             for f in files(\"{glob}\"):\n        \
                 c = len(regex_find(r\"(?i)\\b(TODO|FIXME)\\b\", f[\"text\"]))\n        \
                 if c > 0:\n            \
                     facts.append({{\"measure\": \"{measure}\", \"value\": c, \
             \"subject\": \"file:\" + f[\"path\"], \"path\": f[\"path\"], \
             \"dims\": {{\"language\": \"{lang}\"}}}})\n    \
             return {{\"facts\": facts}}\n"
    )
}

/// Build a gauge [`Collector`] from a gauge's `compute:` block (mirrors
/// `collection.rs::plugin_to_collector`, but always `Gauge` kind).
fn compute_to_collector(gauge: &ResolvedGauge, project_dir: &Path) -> Result<Collector, String> {
    let c: &GaugeComputeConfig = &gauge.compute;
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
    let name = gauge.key.clone();
    let formats = [gauge.key.clone()];
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

/// The script a gauge runs — the embedded text for a built-in, the `entryFile`'s
/// contents for a global/project one. `None` when it can't be read (an `exec` gauge
/// with no readable script, or a missing file).
fn gauge_script_text(gauge: &ResolvedGauge, base: &Path) -> Option<String> {
    if gauge.scope == "built-in" {
        return builtin_metrics()
            .iter()
            .find(|m| m.key == gauge.key)
            .map(|m| m.script.to_string());
    }
    let entry = gauge.compute.entry_file.as_deref()?;
    std::fs::read_to_string(base.join(entry)).ok()
}

/// A fingerprint of the LOGIC that produces a gauge's facts (tsk45): its script
/// text plus the compute knobs and the `emits` allow-list.
///
/// This is what makes a gauge fix actually land. A gauge's facts are only as good as
/// the code that computed them, so when the script changes they are stale — but
/// nothing recomputes them, because the baseline only fires on an EMPTY fold. The
/// result is that you fix a query, the number doesn't move, and nothing tells you
/// why (tsk44: adding inner `#![allow]` to `repo_allow.star` silently no-opped).
/// Stamping this on every capture lets boot spot the drift and re-baseline.
///
/// `None` when the script can't be read — better to skip the check than to
/// re-baseline the whole tree on every boot over an unreadable file.
fn gauge_fingerprint(gauge: &ResolvedGauge, base: &Path) -> Option<String> {
    let script = gauge_script_text(gauge, base)?;
    let c = &gauge.compute;
    // Everything that can change what the gauge produces. `emits` matters because a
    // measure dropped from the allow-list silently stops being recorded.
    let material = format!(
        "v1\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}",
        c.runtime,
        c.input.as_deref().unwrap_or("text"),
        c.args.join(","),
        c.report.as_deref().unwrap_or(""),
        gauge.emits.join(","),
        script,
    );
    Some(crate::blob_store::BlobStore::hash(material.as_bytes()))
}

/// Trust label: in-process tiers are `observed` under a `metric:<key>` source;
/// the `exec` escape hatch is flagged `plugin-exec:<name>` (lower-trust).
fn gauge_source(gauge: &ResolvedGauge, collector: &Collector) -> String {
    use oxplow_collect_plugin::CollectorRuntime;
    if collector.runtime() == CollectorRuntime::Exec {
        format!("plugin-exec:{}", gauge.key)
    } else {
        format!("metric:{}", gauge.key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_config::GaugeComputeConfig;

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

    /// A snapshot on stream 1 carrying `files` as its file rows — the gauge's
    /// SCANNED SET.
    ///
    /// In production `build_file_map` derives the gauge's file map FROM these rows,
    /// so the two are the same set by construction. A `per-path` measure's fold
    /// (tsk41) anchors on them to know which paths a capture restated, so a test
    /// that hands `run_one_gauge` a map must create the matching snapshot — otherwise
    /// the capture restates nothing and its facts never surface.
    async fn snapshot_with_files(
        svc: &Arc<crate::Services>,
        files: &[(&str, oxplow_db::SnapshotStorage)],
    ) -> i64 {
        let snap = svc
            .snapshot_store
            .create_snapshot(oxplow_domain::StreamId::new(1))
            .await
            .unwrap();
        let rows: Vec<oxplow_db::FileSnapshot> = files
            .iter()
            .map(|(path, storage)| oxplow_db::FileSnapshot {
                id: 0,
                stream_id: oxplow_domain::StreamId::new(1),
                path: (*path).to_string(),
                blob_hash: matches!(storage, oxplow_db::SnapshotStorage::Deleted)
                    .then(|| None)
                    .unwrap_or(Some("h".into())),
                size_bytes: 1,
                captured_at: oxplow_domain::Timestamp::now(),
                storage: *storage,
                snapshot_id: Some(snap),
                mtime_ms: None,
            })
            .collect();
        if !rows.is_empty() {
            svc.snapshot_store.capture_batch(rows).await.unwrap();
        }
        snap
    }

    fn starlark_gauge(key: &str, entry_file: &str) -> ResolvedGauge {
        starlark_gauge_emits(key, entry_file, Vec::new())
    }

    /// A project-scope Starlark gauge with an explicit `emits` allow-list.
    fn starlark_gauge_emits(key: &str, entry_file: &str, emits: Vec<String>) -> ResolvedGauge {
        ResolvedGauge {
            key: key.into(),
            title: key.into(),
            trigger: "on-snapshot".into(),
            emits,
            compute: GaugeComputeConfig {
                runtime: "starlark".into(),
                entry_file: Some(entry_file.into()),
                ..Default::default()
            },
            scope: "project".into(),
        }
    }

    #[tokio::test]
    async fn run_one_gauge_records_facts_with_version_and_branch() {
        let (svc, dir) = fixture().await;
        // A tree-derived gauge emitting a per-file `oxplow.ast_hit` FACT.
        std::fs::create_dir_all(dir.path().join("oxplow/metrics")).unwrap();
        std::fs::write(
            dir.path().join("oxplow/metrics/unsafe.star"),
            r#"
def transform(input):
    facts = []
    for f in files("**/*.rs"):
        c = len(ast_query(f["text"], "rust", "(unsafe_block) @u"))
        if c > 0:
            facts.append({"measure": "oxplow.ast_hit", "value": c, "rule": "unsafe_block", "subject": "file:" + f["path"], "path": f["path"], "dims": {"language": "rust"}})
    return {"facts": facts}
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
            effort_id: None,
            scan_kind: "delta",
        };
        // Only src/a.rs has unsafe blocks → one fact recorded.
        let count = svc
            .metrics
            .run_one_gauge(&metric, &ctx, Arc::new(files))
            .await;
        assert_eq!(count, 1);

        let measure = svc
            .fact_store
            .get_measure("oxplow.ast_hit")
            .await
            .unwrap()
            .expect("oxplow.ast_hit seeded by migration");
        let facts = svc.fact_store.facts_for_measure(measure.id).await.unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].value, 2.0, "two unsafe blocks in src/a.rs");
        assert_eq!(facts[0].subject_kind.as_deref(), Some("file"));
        assert_eq!(facts[0].subject_ref.as_deref(), Some("src/a.rs"));
        assert_eq!(facts[0].path.as_deref(), Some("src/a.rs"));
        assert_eq!(facts[0].rule.as_deref(), Some("unsafe_block"));
        // The capture spine carries the run's version + branch + source.
        assert_eq!(facts[0].closest_git_version.as_deref(), Some("abc1234"));
        assert_eq!(facts[0].branch.as_deref(), Some("metrics-substrate"));
        assert_eq!(facts[0].source, "metric:repo.unsafe_blocks");
        assert_eq!(
            facts[0].dims_json.as_deref(),
            Some("{\"language\":\"rust\"}")
        );
    }

    #[tokio::test]
    async fn rescanning_a_fixed_file_supersedes_its_facts_and_drops_the_metric_to_zero() {
        // tsk44's promise ("fixing the last offender must show") under per-path
        // capture scope (tsk41). "Fixed" is expressed by RESCANNING the file with
        // clean content: the path is in the new snapshot, so the new capture
        // restates it, and — emitting no fact — supersedes the stale count with 0.
        // Note the gauge still skips the zero (`if c > 0:`); the scanned set comes
        // from the SNAPSHOT, which is exactly why no zero-emission convention is
        // needed.
        let (svc, _dir) = fixture().await;
        svc.metrics.seed_catalog().await;
        let gauge = builtin_gauge_fixture("oxplow.rust.unsafe_blocks");
        let ctx = |snapshot_id: i64| GaugeRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "on-snapshot",
            snapshot_id: Some(snapshot_id),
            closest_git_version: None,
            git_version_exact: false,
            branch: None,
            effort_id: None,
            scan_kind: "delta",
        };

        // Scan 1: src/a.rs has one unsafe block.
        let s1 =
            snapshot_with_files(&svc, &[("src/a.rs", oxplow_db::SnapshotStorage::Oxplow)]).await;
        let dirty = HashMap::from([(
            "src/a.rs".to_string(),
            "fn a() { unsafe { x(); } }".to_string(),
        )]);
        svc.metrics
            .run_one_gauge(&gauge, &ctx(s1), Arc::new(dirty))
            .await;

        let spec = svc
            .fact_store
            .get_spec("oxplow.rust.unsafe_blocks")
            .await
            .unwrap()
            .expect("ast spec seeded");
        assert_eq!(
            svc.metric_engine.headline_for_spec(&spec).await.unwrap(),
            Some(1.0)
        );

        // Scan 2: src/a.rs is rescanned, now clean → the gauge emits nothing.
        let s2 =
            snapshot_with_files(&svc, &[("src/a.rs", oxplow_db::SnapshotStorage::Oxplow)]).await;
        let clean = HashMap::from([("src/a.rs".to_string(), "fn a() { x(); }".to_string())]);
        svc.metrics
            .run_one_gauge(&gauge, &ctx(s2), Arc::new(clean))
            .await;

        assert_eq!(
            svc.metric_engine.headline_for_spec(&spec).await.unwrap(),
            Some(0.0),
            "rescanning the file supersedes its stale fact — the fix shows"
        );
    }

    /// A file map big enough to count as a whole-tree sweep.
    fn big_corpus(n: usize) -> Arc<HashMap<String, String>> {
        Arc::new(
            (0..n)
                .map(|i| (format!("src/f{i}.rs"), "fn a() {}".to_string()))
                .collect(),
        )
    }

    fn sweep_ctx() -> GaugeRunContext {
        GaugeRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "on-snapshot",
            snapshot_id: None,
            closest_git_version: None,
            git_version_exact: false,
            branch: None,
            effort_id: None,
            scan_kind: "delta",
        }
    }

    #[tokio::test]
    async fn rebuild_metric_baseline_reads_the_whole_repo_end_to_end() {
        // THE test that would have caught all four metrics bugs (tsk47/48/49) without a
        // restart: real files on disk → full-tree snapshot → every gauge → fold →
        // repo-wide headline, driven through the same `rebuild_metric_baseline` boot
        // uses. It runs TWO gauges sharing `oxplow.ast_hit` — the exact shape tsk49
        // hid in — so a single-gauge unit test could not have found it.
        let (svc, dir) = fixture().await;
        svc.metrics.seed_catalog().await;
        svc.metrics
            .set_metrics_enabled(
                &[
                    "oxplow.rust.unsafe_blocks".into(),
                    "oxplow.ts.console_calls".into(),
                ],
                true,
            )
            .await
            .unwrap();

        let write = |rel: &str, body: &str| {
            let p = dir.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write(
            "src/a.rs",
            "fn a() { unsafe { x(); } }\nfn b() { unsafe { y(); } }\n",
        );
        write("src/b.rs", "fn c() { let _z = 1; }\n");
        write("web/app.ts", "console.log(1);\nconsole.error(2);\n");
        write("web/other.ts", "export const x = 1;\n");

        let report = svc.rebuild_metric_baseline(true).await.unwrap();
        assert!(report.ran, "a forced rebuild must run");
        assert!(
            report.failed.is_empty(),
            "no gauge should fail: {:?}",
            report.failed
        );

        // Both read the WHOLE repo from a single baseline — the numbers that read 0
        // (unsafe under semi-additive) / empty (console under the shared-measure bug).
        let unsafe_spec = svc
            .fact_store
            .get_spec("oxplow.rust.unsafe_blocks")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            svc.metric_engine
                .headline_for_spec(&unsafe_spec)
                .await
                .unwrap(),
            Some(2.0),
            "2 unsafe blocks across the tree"
        );
        let console_spec = svc
            .fact_store
            .get_spec("oxplow.ts.console_calls")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            svc.metric_engine
                .headline_for_spec(&console_spec)
                .await
                .unwrap(),
            Some(2.0),
            "console_calls reads the whole repo — the bug that took four restarts"
        );

        // A NON-forced rebuild on the now-warm repo is a no-op: every gauge has
        // scanned the whole tree at its current fingerprint, so nothing needs redoing.
        // This is the guard against the every-boot baseline loop.
        let warm = svc.rebuild_metric_baseline(false).await.unwrap();
        assert!(!warm.ran, "a warm, up-to-date repo must not re-baseline");
        assert_eq!(
            svc.metric_engine
                .headline_for_spec(&console_spec)
                .await
                .unwrap(),
            Some(2.0)
        );
    }

    #[tokio::test]
    async fn rebuild_does_not_fabricate_a_snapshot_on_a_clean_tree() {
        // tsk71: the old rebuild enqueued EVERY path as dirty and captured a
        // full-tree snapshot, which polluted effort file-attribution (edits
        // from other efforts first landed in a snapshot inside whatever effort
        // window was open). The baseline now anchors to the latest existing
        // snapshot and scans the RECONSTRUCTED tree — so a rebuild over an
        // unchanged tree must not create any snapshot at all.
        let (svc, dir) = fixture().await;
        svc.metrics.seed_catalog().await;
        svc.metrics
            .set_metrics_enabled(&["oxplow.rust.unsafe_blocks".into()], true)
            .await
            .unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "fn a() { unsafe { x(); } }\n").unwrap();

        let first = svc.rebuild_metric_baseline(true).await.unwrap();
        assert!(first.ran);
        assert!(first.failed.is_empty(), "{:?}", first.failed);
        let latest_after_first = svc
            .snapshot_store
            .latest_snapshot_id_for_stream(oxplow_domain::StreamId::new(1))
            .await
            .unwrap();

        // Second forced rebuild: nothing on disk changed → no new snapshot,
        // and the kind-scoped idempotency guard skips the re-scan.
        let second = svc.rebuild_metric_baseline(true).await.unwrap();
        assert!(second.ran);
        let latest_after_second = svc
            .snapshot_store
            .latest_snapshot_id_for_stream(oxplow_domain::StreamId::new(1))
            .await
            .unwrap();
        assert_eq!(
            latest_after_first, latest_after_second,
            "a rebuild over an unchanged tree must not fabricate a snapshot"
        );
        assert_eq!(second.snapshot_id, latest_after_first);
    }

    #[tokio::test]
    async fn a_delta_only_gauge_needs_a_baseline_even_if_a_sibling_filled_the_shared_measure() {
        // tsk49, the exact live bug. `oxplow.ast_hit` is ONE measure shared by 10 idiom
        // gauges. `unsafe_blocks` (cheap) completed its full-tree scan, so the measure
        // has facts — but `console_calls` (heavy, timed out under the old budget) only
        // ever ran on small deltas. A measure-level "is it empty" check says "done" and
        // console_calls reads empty forever. The baseline question must be per-gauge.
        let (svc, _dir) = fixture().await;
        svc.metrics.seed_catalog().await;
        // A gauge is only in `resolved_gauges` when its metric is enabled — enable the
        // two built-ins this test drives.
        svc.metrics
            .set_metrics_enabled(
                &[
                    "oxplow.rust.unsafe_blocks".into(),
                    "oxplow.ts.console_calls".into(),
                ],
                true,
            )
            .await
            .unwrap();
        let ctx = |snapshot_id: i64| GaugeRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "on-snapshot",
            snapshot_id: Some(snapshot_id),
            closest_git_version: None,
            git_version_exact: false,
            branch: None,
            effort_id: None,
            scan_kind: "delta",
        };
        let src = "pub fn f() {\n    unsafe { g(); }\n    console.log(x);\n}\n".to_string();

        // unsafe_blocks completes its BASELINE (a `scan_kind = 'full'` capture,
        // tsk71). It shares oxplow.ast_hit with console_calls.
        let big = snapshot_with_files(
            &svc,
            &(0..150)
                .map(|i| (format!("f{i}.rs"), oxplow_db::SnapshotStorage::Oxplow))
                .collect::<Vec<_>>()
                .iter()
                .map(|(p, s)| (p.as_str(), *s))
                .collect::<Vec<_>>(),
        )
        .await;
        let files = Arc::new(HashMap::from([("f0.rs".to_string(), src)]));
        let mut full_ctx = ctx(big);
        full_ctx.scan_kind = "full";
        svc.metrics
            .run_one_gauge(
                &builtin_gauge_fixture("oxplow.rust.unsafe_blocks"),
                &full_ctx,
                files,
            )
            .await;

        // console_calls has only ever run on a tiny delta.
        let small =
            snapshot_with_files(&svc, &[("f0.tsx", oxplow_db::SnapshotStorage::Oxplow)]).await;
        svc.metrics
            .run_one_gauge(
                &builtin_gauge_fixture("oxplow.ts.console_calls"),
                &ctx(small),
                Arc::new(HashMap::from([("f0.tsx".to_string(), "ok".to_string())])),
            )
            .await;

        let needing = svc.metrics.gauges_needing_baseline(1).await;
        assert!(
            needing.contains(&"oxplow.ts.console_calls".to_string()),
            "the delta-only gauge must still need a baseline; got {needing:?}"
        );
        assert!(
            !needing.contains(&"oxplow.rust.unsafe_blocks".to_string()),
            "the gauge that scanned the full tree must NOT need one; got {needing:?}"
        );
    }

    #[tokio::test]
    async fn a_whole_tree_sweep_is_visible_as_a_background_task() {
        // tsk48. The baseline pegs a core for minutes and used to report NOTHING —
        // "why is oxplow eating CPU?" had no answer, and when it went wrong I could
        // only find out by reading SQL by hand.
        let (svc, _dir) = fixture().await;
        svc.metrics.seed_catalog().await;
        let gauge = builtin_gauge_fixture("oxplow.rust.unsafe_blocks");

        svc.metrics
            .run_gauge_sweep(&[gauge], &sweep_ctx(), big_corpus(120), 1)
            .await;

        let task = svc
            .background_tasks
            .list_running()
            .into_iter()
            .find(|t| t.kind == crate::background_task::BackgroundTaskKind::Metrics)
            .expect("a whole-tree sweep must surface as a Metrics background task");
        assert_eq!(
            task.status,
            crate::background_task::BackgroundTaskStatus::Done
        );
        assert!(task.label.contains("metrics"), "got {:?}", task.label);
    }

    #[tokio::test]
    async fn an_ordinary_delta_sweep_is_not_tracked() {
        // A per-commit delta finishes in milliseconds — tracking it would be noise.
        let (svc, _dir) = fixture().await;
        svc.metrics.seed_catalog().await;
        let gauge = builtin_gauge_fixture("oxplow.rust.unsafe_blocks");

        svc.metrics
            .run_gauge_sweep(&[gauge], &sweep_ctx(), big_corpus(3), 1)
            .await;

        assert!(
            !svc.background_tasks
                .list_running()
                .iter()
                .any(|t| t.kind == crate::background_task::BackgroundTaskKind::Metrics),
            "a 3-file delta must not raise a background task"
        );
    }

    #[tokio::test]
    async fn a_sweep_with_a_failing_gauge_fails_the_task_rather_than_quietly_succeeding() {
        // The whole point of tsk47/tsk48: a gauge that blows up leaves its metric
        // reading stale or empty. Reporting the sweep as "done" would hide exactly the
        // failure that let two built-in metrics read empty for weeks.
        let (svc, dir) = fixture().await;
        svc.metrics.seed_catalog().await;

        let script = dir.path().join("oxplow/metrics/boom.star");
        std::fs::create_dir_all(script.parent().unwrap()).unwrap();
        std::fs::write(&script, "def transform(input):\n    fail(\"boom\")\n").unwrap();
        let gauge = starlark_gauge_emits(
            "acme.boom",
            "oxplow/metrics/boom.star",
            vec!["oxplow.todo".to_string()],
        );

        svc.metrics
            .run_gauge_sweep(&[gauge], &sweep_ctx(), big_corpus(120), 1)
            .await;

        let task = svc
            .background_tasks
            .list_running()
            .into_iter()
            .find(|t| t.kind == crate::background_task::BackgroundTaskKind::Metrics)
            .expect("tracked sweep");
        assert_eq!(
            task.status,
            crate::background_task::BackgroundTaskStatus::Failed,
            "a failing gauge must fail the sweep, not vanish into a log line"
        );
        assert!(
            task.error
                .as_deref()
                .unwrap_or_default()
                .contains("acme.boom"),
            "the failure must name the gauge; got {:?}",
            task.error
        );
    }

    #[tokio::test]
    async fn changing_a_gauge_script_marks_it_stale_so_the_fix_actually_lands() {
        // tsk45. The trap this closes: you fix a gauge's query, the metric doesn't
        // move, and nothing tells you why — because its old facts aren't EMPTY, just
        // WRONG, and the baseline only fires on an empty fold. (Real case, tsk44:
        // teaching repo_allow.star to also match inner `#![allow]` silently no-opped.)
        let (svc, dir) = fixture().await;
        svc.metrics.seed_catalog().await;

        let script = dir.path().join("oxplow/metrics/g.star");
        std::fs::create_dir_all(script.parent().unwrap()).unwrap();
        let write = |body: &str| std::fs::write(&script, body).unwrap();
        write(
            "def transform(input):\n    \
             return {\"facts\": [{\"measure\": \"oxplow.todo\", \"value\": 1.0, \
             \"subject\": \"file:a.rs\", \"path\": \"a.rs\"}]}\n",
        );
        let gauge = starlark_gauge_emits(
            "acme.g",
            "oxplow/metrics/g.star",
            vec!["oxplow.todo".to_string()],
        );

        let snap = snapshot_with_files(&svc, &[("a.rs", oxplow_db::SnapshotStorage::Oxplow)]).await;
        let ctx = GaugeRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "on-snapshot",
            snapshot_id: Some(snap),
            closest_git_version: None,
            git_version_exact: false,
            branch: None,
            effort_id: None,
            scan_kind: "delta",
        };
        let files = Arc::new(HashMap::from([("a.rs".to_string(), "x".to_string())]));
        svc.metrics.run_one_gauge(&gauge, &ctx, files.clone()).await;

        // Same script → the recorded fingerprint still matches → nothing to redo.
        assert!(
            !svc.metrics.gauge_is_stale(&gauge, 1).await,
            "an unchanged gauge must not force a re-baseline on every boot"
        );

        // Now the author fixes the gauge's logic (a different value).
        write(
            "def transform(input):\n    \
             return {\"facts\": [{\"measure\": \"oxplow.todo\", \"value\": 5.0, \
             \"subject\": \"file:a.rs\", \"path\": \"a.rs\"}]}\n",
        );
        assert!(
            svc.metrics.gauge_is_stale(&gauge, 1).await,
            "a changed script must be detected as stale — otherwise the fix no-ops"
        );
    }

    #[tokio::test]
    async fn a_delta_rescan_updates_the_repo_wide_total_incrementally() {
        // THE bug, end to end (tsk41). Baseline the whole tree, then rescan only the
        // ONE file a commit changed. The metric must report the REPO-WIDE total — not
        // the delta — with the untouched files' facts carried forward from the
        // baseline, and the rescanned file's stale facts superseded.
        //
        // This is what read 0-instead-of-15: the old semi-additive fold took "the last
        // capture", which after the baseline is only ever a handful of changed files.
        let (svc, _dir) = fixture().await;
        svc.metrics.seed_catalog().await;
        let gauge = builtin_gauge_fixture("oxplow.rust.unsafe_blocks");
        let ctx = |snapshot_id: i64| GaugeRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "on-snapshot",
            snapshot_id: Some(snapshot_id),
            closest_git_version: None,
            git_version_exact: false,
            branch: None,
            effort_id: None,
            scan_kind: "delta",
        };
        let unsafe_n = |n: usize| {
            let body = "unsafe { x(); } ".repeat(n);
            format!("fn f() {{ {body} }}")
        };

        // BASELINE: a full-tree snapshot — a.rs has 2 unsafe blocks, b.rs 1, c.rs 0.
        let base = snapshot_with_files(
            &svc,
            &[
                ("a.rs", oxplow_db::SnapshotStorage::Oxplow),
                ("b.rs", oxplow_db::SnapshotStorage::Oxplow),
                ("c.rs", oxplow_db::SnapshotStorage::Oxplow),
            ],
        )
        .await;
        let full_tree = HashMap::from([
            ("a.rs".to_string(), unsafe_n(2)),
            ("b.rs".to_string(), unsafe_n(1)),
            ("c.rs".to_string(), unsafe_n(0)),
        ]);
        svc.metrics
            .run_one_gauge(&gauge, &ctx(base), Arc::new(full_tree))
            .await;

        let spec = svc
            .fact_store
            .get_spec("oxplow.rust.unsafe_blocks")
            .await
            .unwrap()
            .expect("ast spec seeded");
        assert_eq!(
            svc.metric_engine.headline_for_spec(&spec).await.unwrap(),
            Some(3.0),
            "the baseline reads the whole repo: 2 + 1 + 0"
        );

        // DELTA: a commit touched ONLY a.rs, fixing one of its two unsafe blocks. The
        // snapshot — and therefore the gauge's file map — lists just that file.
        let delta =
            snapshot_with_files(&svc, &[("a.rs", oxplow_db::SnapshotStorage::Oxplow)]).await;
        let changed = HashMap::from([("a.rs".to_string(), unsafe_n(1))]);
        svc.metrics
            .run_one_gauge(&gauge, &ctx(delta), Arc::new(changed))
            .await;

        assert_eq!(
            svc.metric_engine.headline_for_spec(&spec).await.unwrap(),
            Some(2.0),
            "repo-wide total moved by exactly a.rs's delta (2→1); b.rs's 1 carried \
             forward from the baseline even though it was never rescanned"
        );
    }

    #[tokio::test]
    async fn a_gauge_run_that_scanned_nothing_supersedes_nothing() {
        // The other half of per-path (tsk41), and the exact bug we fixed: a delta
        // capture that restated NO paths must leave the metric alone. Under the old
        // semi-additive reading, this empty capture zero-filled the series and the
        // headline read 0 — which is how `oxplow.rust.unsafe_blocks` reported 0 while
        // the repo had 15 unsafe blocks.
        let (svc, _dir) = fixture().await;
        svc.metrics.seed_catalog().await;
        let gauge = builtin_gauge_fixture("oxplow.rust.unsafe_blocks");
        let ctx = |snapshot_id: i64| GaugeRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "on-snapshot",
            snapshot_id: Some(snapshot_id),
            closest_git_version: None,
            git_version_exact: false,
            branch: None,
            effort_id: None,
            scan_kind: "delta",
        };

        let s1 =
            snapshot_with_files(&svc, &[("src/a.rs", oxplow_db::SnapshotStorage::Oxplow)]).await;
        let dirty = HashMap::from([(
            "src/a.rs".to_string(),
            "fn a() { unsafe { x(); } }".to_string(),
        )]);
        svc.metrics
            .run_one_gauge(&gauge, &ctx(s1), Arc::new(dirty))
            .await;

        // A later commit touched nothing this gauge scans: an empty snapshot + an
        // empty file map.
        let s2 = snapshot_with_files(&svc, &[]).await;
        svc.metrics
            .run_one_gauge(&gauge, &ctx(s2), Arc::new(HashMap::new()))
            .await;

        let spec = svc
            .fact_store
            .get_spec("oxplow.rust.unsafe_blocks")
            .await
            .unwrap()
            .expect("ast spec seeded");
        assert_eq!(
            svc.metric_engine.headline_for_spec(&spec).await.unwrap(),
            Some(1.0),
            "scanning nothing must NOT zero the repo"
        );
    }

    #[tokio::test]
    async fn effort_complete_gauge_capture_is_effort_stamped() {
        // tsk43: the on-effort-complete trigger KNOWS its producing effort —
        // `record_gauge_facts` stamps the capture's `effort_id` so the T-D
        // attribution spine (`captures_for_effort`) sees the run. Snapshot scans
        // (the other fixtures, `effort_id: None`) stay unstamped.
        use oxplow_domain::stores::TaskStore as _;
        use oxplow_domain::{Task, TaskActorKind, TaskAuthor, TaskId, TaskPriority, TaskStatus};
        let (svc, dir) = fixture().await;
        // A real effort row (the capture's effort_id is a foreign key).
        let now = oxplow_domain::Timestamp::now();
        let thread = ThreadId::new(1);
        let task = svc
            .task_store
            .insert(&Task {
                id: TaskId::placeholder(),
                thread_id: Some(thread),
                parent_id: None,
                title: "t".into(),
                description: String::new(),
                status: TaskStatus::InProgress,
                priority: TaskPriority::Medium,
                sort_index: 0,
                created_by: TaskActorKind::User,
                created_at: now,
                updated_at: now,
                completed_at: None,
                deleted_at: None,
                note_count: 0,
                author: Some(TaskAuthor::User),
            })
            .await
            .unwrap();
        let effort = svc.effort_store.start(task, &thread, None).await.unwrap();

        std::fs::create_dir_all(dir.path().join("oxplow/metrics")).unwrap();
        std::fs::write(
            dir.path().join("oxplow/metrics/eff.star"),
            r#"
def transform(input):
    return {"facts": [{"measure": "oxplow.ast_hit", "value": 1, "rule": "eff", "subject": "tree:."}]}
"#,
        )
        .unwrap();
        let metric = starlark_gauge("acme.effgauge", "oxplow/metrics/eff.star");
        let ctx = GaugeRunContext {
            stream_val: 1,
            thread_id: Some(1),
            trigger: "on-effort-complete",
            snapshot_id: None,
            closest_git_version: None,
            git_version_exact: false,
            branch: None,
            effort_id: Some(effort.id.value()),
            scan_kind: "delta",
        };
        let count = svc
            .metrics
            .run_one_gauge(&metric, &ctx, Arc::new(HashMap::new()))
            .await;
        assert_eq!(count, 1);
        let caps = svc
            .fact_store
            .captures_for_effort(effort.id.value())
            .await
            .unwrap();
        assert_eq!(caps.len(), 1, "the capture is stamped with the effort");
        assert_eq!(caps[0].producer, "acme.effgauge");
    }

    #[tokio::test]
    async fn run_one_gauge_records_per_item_facts() {
        let (svc, dir) = fixture().await;
        // A gauge emitting a per-function `oxplow.fn_length` FACT — the located
        // items behind the metric (the drill-in reads them via findings_for_spec).
        std::fs::create_dir_all(dir.path().join("oxplow/metrics")).unwrap();
        std::fs::write(
            dir.path().join("oxplow/metrics/longfns.star"),
            r#"
def transform(input):
    facts = []
    for f in files("**/*.rs"):
        for m in code_metrics(f["text"], "rust"):
            facts.append({"measure": "oxplow.fn_length", "value": m["length"], "subject": "symbol:" + f["path"] + "::" + m["name"], "path": f["path"], "line": m["start_line"], "dims": {"language": "rust"}})
    return {"facts": facts}
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
            effort_id: None,
            scan_kind: "delta",
        };
        let count = svc
            .metrics
            .run_one_gauge(&metric, &ctx, Arc::new(files))
            .await;
        assert_eq!(count, 1, "one function → one fact");

        let measure = svc
            .fact_store
            .get_measure("oxplow.fn_length")
            .await
            .unwrap()
            .expect("oxplow.fn_length seeded by migration");
        let facts = svc.fact_store.facts_for_measure(measure.id).await.unwrap();
        assert_eq!(facts.len(), 1, "one function → one fact");
        assert_eq!(facts[0].path.as_deref(), Some("src/a.rs"));
        assert_eq!(facts[0].subject_kind.as_deref(), Some("symbol"));
        assert_eq!(facts[0].subject_ref.as_deref(), Some("src/a.rs::big"));
        assert!(facts[0].value >= 3.0);
    }

    /// A built-in-scope `ResolvedGauge` — `run_one_gauge` builds its collector
    /// from the embedded script (never a project-disk file).
    fn builtin_gauge_fixture(key: &str) -> ResolvedGauge {
        super::builtin_gauge(key, "on-snapshot")
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
    async fn code_gauge_facts_reaggregate_to_the_expected_headline() {
        // The keystone proof of the inversion (epic tsk12): a metric SPEC computed
        // over the per-item FACTS the gauge emitted == the expected gauge total,
        // for every bundled code metric. This is what let the reads flip to the
        // engine (T-C3) and the baked sample be removed (T-C3b).
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
            effort_id: None,
            scan_kind: "delta",
        };
        for key in [
            "oxplow.fn_count",
            "oxplow.high_complexity_fns",
            "oxplow.long_functions",
            "oxplow.todos",
        ] {
            svc.metrics
                .run_one_gauge(&builtin_gauge_fixture(key), &ctx, files.clone())
                .await;
        }

        let engine = crate::metric_engine::MetricEngine::new((*svc.fact_store).clone());
        for (key, expected) in [
            ("oxplow.fn_count", 4.0),
            ("oxplow.high_complexity_fns", 1.0),
            ("oxplow.long_functions", 1.0),
            ("oxplow.todos", 2.0),
        ] {
            let spec = svc
                .fact_store
                .get_spec(key)
                .await
                .unwrap()
                .unwrap_or_else(|| panic!("{key} spec seeded"));
            let engine_headline = engine.headline_for_spec(&spec).await.unwrap();
            assert_eq!(
                engine_headline,
                Some(expected),
                "{key}: facts re-aggregated through the engine must equal the gauge total",
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
            effort_id: None,
            scan_kind: "delta",
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
    async fn gauge_facts_outside_the_emits_allow_list_are_dropped() {
        // A config gauge's `emits` is its contract: even a fact on a DEFINED
        // catalog measure is dropped if the gauge didn't declare it. Here the
        // gauge emits only `oxplow.complexity`; a sibling fact on the (also
        // defined) `oxplow.fn_length` measure is dropped for being off-contract.
        let (svc, dir) = fixture().await;
        std::fs::create_dir_all(dir.path().join("oxplow/gauges")).unwrap();
        std::fs::write(
            dir.path().join("oxplow/gauges/emits.star"),
            r#"
def transform(input):
    return {"facts": [
        {"measure": "oxplow.complexity", "value": 5, "subject": "symbol:src/a.rs::foo"},
        {"measure": "oxplow.fn_length", "value": 9, "subject": "symbol:src/a.rs::bar"},
    ]}
"#,
        )
        .unwrap();
        let gauge = starlark_gauge_emits(
            "acme.only_complexity",
            "oxplow/gauges/emits.star",
            vec!["oxplow.complexity".into()],
        );
        let ctx = GaugeRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "manual",
            snapshot_id: None,
            closest_git_version: None,
            git_version_exact: false,
            branch: None,
            effort_id: None,
            scan_kind: "delta",
        };
        svc.metrics
            .run_one_gauge(&gauge, &ctx, Arc::new(HashMap::new()))
            .await;

        let complexity = svc
            .fact_store
            .get_measure("oxplow.complexity")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            svc.fact_store
                .facts_for_measure(complexity.id)
                .await
                .unwrap()
                .len(),
            1,
            "the declared-measure fact is written"
        );
        let fn_length = svc
            .fact_store
            .get_measure("oxplow.fn_length")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            svc.fact_store
                .facts_for_measure(fn_length.id)
                .await
                .unwrap()
                .len(),
            0,
            "the off-contract fact (defined but not in `emits`) is dropped"
        );
    }

    #[tokio::test]
    async fn gauge_facts_carry_ratio_components() {
        // A gauge may emit `num`/`den` on a ratio-base fact so a `ratio` spec
        // re-derives Σnum/Σden exactly (coverage %, pass rate) rather than
        // averaging pre-divided values. Prove they round-trip onto the fact row.
        let (svc, dir) = fixture().await;
        std::fs::create_dir_all(dir.path().join("oxplow/gauges")).unwrap();
        std::fs::write(
            dir.path().join("oxplow/gauges/ratio.star"),
            r#"
def transform(input):
    return {"facts": [
        {"measure": "oxplow.complexity", "value": 0.5, "num": 3, "den": 6,
         "subject": "file:src/a.rs"},
    ]}
"#,
        )
        .unwrap();
        let gauge = starlark_gauge_emits(
            "acme.ratio",
            "oxplow/gauges/ratio.star",
            vec!["oxplow.complexity".into()],
        );
        let ctx = GaugeRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "manual",
            snapshot_id: None,
            closest_git_version: None,
            git_version_exact: false,
            branch: None,
            effort_id: None,
            scan_kind: "delta",
        };
        svc.metrics
            .run_one_gauge(&gauge, &ctx, Arc::new(HashMap::new()))
            .await;

        let m = svc
            .fact_store
            .get_measure("oxplow.complexity")
            .await
            .unwrap()
            .unwrap();
        let rows = svc.fact_store.facts_for_measure(m.id).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].numerator, Some(3.0));
        assert_eq!(rows[0].denominator, Some(6.0));
    }

    #[tokio::test]
    async fn per_language_gauge_facts_reaggregate_through_the_spec() {
        // tsk30: the per-language idiom gauges emit per-file `oxplow.ast_hit`
        // facts (rule-tagged); each metric is a Sum(oxplow.ast_hit) spec filtered
        // by rule. Prove every emitted-fact stream re-aggregates through its spec
        // to a positive headline (the exact per-idiom counts are pinned by the
        // collect-plugin golden tests). One capture per gauge; idioms share the
        // measure but never collide (the spec filters by rule).
        let (svc, _dir) = fixture().await;
        svc.metrics.seed_catalog().await;

        let mut corpus = HashMap::new();
        corpus.insert(
            "src/a.rs".to_string(),
            "fn a() {\n    unsafe { foo(); }\n    let x = maybe().unwrap();\n    \
             let y = maybe().expect(\"nope\");\n    if x { panic!(\"boom\"); }\n}\n\
             fn b() {\n    unsafe { bar(); }\n    todo!();\n    std::panic!(\"q\");\n}\n"
                .to_string(),
        );
        corpus.insert(
            "src/a.ts".to_string(),
            "// @ts-ignore\nfunction f(x: any): any {\n    console.log(x);\n    \
             window.console.error(x);\n    const y = x!.foo;\n    return y;\n}\n\
             const g = (a: any) => a!;\n"
                .to_string(),
        );
        corpus.insert(
            "src/core.clj".to_string(),
            ";; TODO\n(defn add [a b] (+ a b))\n(defn- helper [] :ok)\n(def x 1)\n(let [defn 1] defn)\n"
                .to_string(),
        );
        corpus.insert(
            "src/Service.cs".to_string(),
            "namespace Acme {\n  class Service {\n    public void Run(int x) {\n      \
             try { Work(); } catch (System.Exception) { }\n      var r = FetchAsync().Result;\n      \
             _task.Wait();\n      System.Action w = _task.Wait;\n    }\n  }\n}\n"
                .to_string(),
        );
        let files = Arc::new(corpus);
        let ctx = GaugeRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "on-snapshot",
            snapshot_id: Some(9),
            closest_git_version: Some("def5678".into()),
            git_version_exact: true,
            branch: Some("main".into()),
            effort_id: None,
            scan_kind: "delta",
        };

        let keys = [
            "oxplow.rust.unsafe_blocks",
            "oxplow.rust.unwrap_expect_calls",
            "oxplow.rust.panic_macros",
            "oxplow.ts.any_usage",
            "oxplow.ts.non_null_assertions",
            "oxplow.ts.console_calls",
            "oxplow.ts.ts_ignore",
            "oxplow.clojure.defn_count",
            "oxplow.csharp.empty_catch",
            "oxplow.csharp.blocking_async_calls",
        ];
        for key in keys {
            svc.metrics
                .run_one_gauge(&builtin_gauge_fixture(key), &ctx, files.clone())
                .await;
        }

        let engine = crate::metric_engine::MetricEngine::new((*svc.fact_store).clone());
        for key in keys {
            let spec = svc
                .fact_store
                .get_spec(key)
                .await
                .unwrap()
                .unwrap_or_else(|| panic!("{key} ast spec seeded"));
            let engine_headline = engine
                .headline_for_spec(&spec)
                .await
                .unwrap()
                .unwrap_or(0.0);
            assert!(
                engine_headline > 0.0,
                "{key}: Sum(oxplow.ast_hit) filtered by rule re-aggregates the \
                 emitted facts to a positive headline",
            );
        }
    }

    #[tokio::test]
    async fn gauge_facts_slice_by_the_conformed_language_dimension() {
        // The conformed catalog declares `oxplow.language` (V43) and
        // list_dimensions advertises it — the bundled gauges' facts must be
        // sliceable by it (group_by / dim_eq), with bare `language` kept as a
        // legacy alias for pre-rename facts and the Explorer's declared dims.
        let (svc, _dir) = fixture().await;
        svc.metrics.seed_catalog().await;
        let mut corpus = HashMap::new();
        corpus.insert(
            "src/lib.rs".to_string(),
            "pub fn f() {\n    unsafe { std::ptr::read(std::ptr::null::<u8>()); }\n}\n".to_string(),
        );
        let files = Arc::new(corpus);
        // The snapshot IS the gauge's scanned set (tsk41) — create it to match the map.
        let snap =
            snapshot_with_files(&svc, &[("src/lib.rs", oxplow_db::SnapshotStorage::Oxplow)]).await;
        let ctx = GaugeRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "on-snapshot",
            snapshot_id: Some(snap),
            closest_git_version: None,
            git_version_exact: false,
            branch: Some("main".into()),
            effort_id: None,
            scan_kind: "delta",
        };
        svc.metrics
            .run_one_gauge(
                &builtin_gauge_fixture("oxplow.rust.unsafe_blocks"),
                &ctx,
                files.clone(),
            )
            .await;

        let engine = crate::metric_engine::MetricEngine::new((*svc.fact_store).clone());
        let spec = svc
            .fact_store
            .get_spec("oxplow.rust.unsafe_blocks")
            .await
            .unwrap()
            .unwrap();
        let rollup = engine
            .rollup_for_spec(&spec, "oxplow.language")
            .await
            .unwrap();
        assert_eq!(rollup.len(), 1, "one language group, got {rollup:?}");
        assert_eq!(rollup[0].key, "rust");
        // The bare key still slices identically.
        let bare = engine.rollup_for_spec(&spec, "language").await.unwrap();
        assert_eq!(bare, rollup);
    }

    #[tokio::test]
    async fn seed_catalog_seeds_producer_specs() {
        // T-B: the always-on producer metrics are seeded as `metric_spec`s beside
        // the built-in gauge specs, over the V43/V46 measures.
        let (svc, _dir) = fixture().await;
        svc.metrics.seed_catalog().await;
        let spec = svc
            .fact_store
            .get_spec("oxplow.coverage.abs_pct")
            .await
            .unwrap()
            .expect("producer spec seeded");
        assert_eq!(spec.source_measure.as_deref(), Some("oxplow.coverage"));
        assert_eq!(spec.aggregation, "ratio");
        // The new V46 measures exist for the producers with no prior home.
        for key in ["oxplow.turn", "oxplow.task_effort", "oxplow.nudge"] {
            assert!(
                svc.fact_store.get_measure(key).await.unwrap().is_some(),
                "{key} measure seeded by V46"
            );
        }
    }

    #[tokio::test]
    async fn seed_catalog_upserts_configured_metric_specs() {
        // T-E2: config `metrics:` entries seed metric SPECS (the legacy
        // definition seeding is gone).
        let (svc, dir) = fixture().await;
        std::fs::write(
            oxplow_config::config_path(dir.path()),
            "metrics:\n  - key: repo.loc\n    title: \"lines\"\n    sourceMeasure: acme.lines\n    aggregation: sum\n",
        )
        .unwrap();
        svc.reload_config_from_disk().unwrap();
        svc.metrics.seed_catalog().await;
        let spec = svc
            .fact_store
            .get_spec("repo.loc")
            .await
            .unwrap()
            .expect("seeded");
        assert_eq!(spec.display_kind, "gauge"); // displayKind defaults to gauge
        assert_eq!(spec.aggregation, "sum");
        assert_eq!(spec.scope, "project");
        assert_eq!(spec.source_measure.as_deref(), Some("acme.lines"));
    }

    #[tokio::test]
    async fn seed_catalog_applies_use_overrides_to_builtin_specs() {
        // A project `use:` of a built-in resolves WITH the user's threshold
        // overrides (the Catalog inline target editor writes exactly this) —
        // they must land on the persisted metric_spec, not be dropped by the
        // built-in skip: the Detail page target, delta_vs_target, and the
        // warn/fail findings all read the spec row.
        let (svc, dir) = fixture().await;
        std::fs::write(
            oxplow_config::config_path(dir.path()),
            "metrics:\n  - use: oxplow.todos\n    target: 5\n    warnAt: 8\n    failAt: 13\n",
        )
        .unwrap();
        svc.reload_config_from_disk().unwrap();
        svc.metrics.seed_catalog().await;
        let spec = svc
            .fact_store
            .get_spec("oxplow.todos")
            .await
            .unwrap()
            .expect("seeded");
        assert_eq!(spec.scope, "built-in");
        assert_eq!(spec.target, Some(5.0));
        assert_eq!(spec.warn_at, Some(8.0));
        assert_eq!(spec.fail_at, Some(13.0));
        // The structural spec survives the override re-seed.
        assert_eq!(spec.source_measure.as_deref(), Some("oxplow.todo"));
        assert_eq!(spec.aggregation, "count");
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
    async fn seed_catalog_threads_promote_flag_to_dimension_row() {
        let (svc, dir) = fixture().await;
        std::fs::write(
            oxplow_config::config_path(dir.path()),
            "dimensions:\n  - key: acme.hot\n    label: Hot\n    promote: true\n  \
             - key: acme.cold\n    label: Cold\n",
        )
        .unwrap();
        svc.reload_config_from_disk().unwrap();
        svc.metrics.seed_catalog().await;

        let dims = svc.fact_store.list_dimensions().await.unwrap();
        let hot = dims
            .iter()
            .find(|d| d.key == "acme.hot")
            .expect("hot seeded");
        let cold = dims
            .iter()
            .find(|d| d.key == "acme.cold")
            .expect("cold seeded");
        assert!(hot.promoted, "promote: true must reach the dimension row");
        assert!(!cold.promoted, "unset promote defaults to false");
    }

    #[tokio::test]
    async fn scaffold_measure_writes_config_and_seeds_catalog() {
        let (svc, dir) = fixture().await;
        let key = svc
            .metrics
            .scaffold_measure(
                MeasureEntry {
                    key: Some("acme.api_latency".into()),
                    title: Some("API latency".into()),
                    unit: Some("ms".into()),
                    subject_kind: Some("endpoint".into()),
                    temporal_semantics: Some("non-additive".into()),
                    ..Default::default()
                },
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
            .scaffold_measure(
                MeasureEntry {
                    key: Some("oxplow.x".into()),
                    ..Default::default()
                },
                None
            )
            .await
            .is_err());
        assert!(svc
            .metrics
            .scaffold_measure(
                MeasureEntry {
                    key: Some("acme.api_latency".into()),
                    ..Default::default()
                },
                None
            )
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

        // Seeding registers the SPEC (T-E2: the legacy definition seeding is
        // gone); the resolved config carries the project's target override into
        // the catalog entry.
        svc.metrics.seed_catalog().await;
        let spec = svc
            .fact_store
            .get_spec("oxplow.rust.unsafe_blocks")
            .await
            .unwrap()
            .expect("seeded");
        assert_eq!(spec.scope, "built-in");
        let cat = svc.metrics.catalog().await;
        let entry = cat
            .iter()
            .find(|e| e.key == "oxplow.rust.unsafe_blocks")
            .expect("catalog entry");
        assert_eq!(entry.target, Some(3.0), "project override merged");

        // Running it executes the EMBEDDED script (no project-disk file). With no
        // snapshot the file map is empty, so the facts-only gauge cleanly yields 0
        // facts and runs without error (the read flip made it facts-only, T-C3b).
        let count = svc
            .metrics
            .run_metric_by_key("oxplow.rust.unsafe_blocks", None)
            .await
            .unwrap();
        assert_eq!(count, 0, "empty snapshot → no facts, runs without error");
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
        // Matches the seeded spec's category (builtin_ast_specs), so the Catalog
        // agrees with the spec catalog it toggles (tsk46).
        assert_eq!(entry.category.as_deref(), Some("static-quality"));

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
        let spec = svc
            .fact_store
            .get_spec("oxplow.rust.unsafe_blocks")
            .await
            .unwrap()
            .expect("spec seeded on enable (T-E2)");
        assert_eq!(spec.scope, "built-in");
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
            // tsk31: every metric is toggleable now (no "always on" class), and
            // producers are enabled by default.
            assert!(e.toggleable, "{key} is toggleable");
            assert!(e.enabled, "{key} enabled by default");
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

        // Simulate a producer (or external plugin) seeding a SPEC directly, the
        // way seed_catalog does for the always-on producers at boot (T-E2: the
        // catalog's tail sweep reads the spec catalog, not legacy definitions).
        let mut spec = oxplow_db::NewMetricSpec::base(
            "agent.tokens.total",
            "Total tokens",
            "oxplow.tokens",
            "sum",
        );
        spec.display_kind = "gauge".into();
        spec.category = Some("operational".to_string());
        svc.fact_store.upsert_spec(spec).await.unwrap();

        let cat = svc.metrics.catalog().await;
        let entry = cat
            .iter()
            .find(|e| e.key == "agent.tokens.total")
            .expect("producer-seeded metric is in the catalog");
        // tsk31: producers are toggleable now, enabled by default.
        assert!(entry.toggleable, "producers are toggleable");
        assert!(entry.enabled, "producers read as enabled by default");
        assert_eq!(entry.category.as_deref(), Some("operational"));

        // A toggleable code gauge still coexists in the same listing.
        assert!(
            cat.iter()
                .any(|e| e.key == "oxplow.rust.unsafe_blocks" && e.toggleable),
            "code gauges still present and toggleable"
        );
    }

    #[tokio::test]
    async fn disabling_a_producer_prunes_its_spec_and_writes_a_marker() {
        let (svc, dir) = fixture().await;
        // Boot-seed the producer specs so there's a row to prune.
        svc.metrics.seed_catalog().await;
        assert!(
            svc.fact_store
                .get_spec("agent.tokens.total")
                .await
                .unwrap()
                .is_some(),
            "producer spec seeded by default"
        );

        // Disable the producer: catalog reads it off, config carries a marker, and
        // the spec is pruned so all spec-driven reads go empty.
        svc.metrics
            .set_metric_enabled("agent.tokens.total", false)
            .await
            .unwrap();
        let entry = svc
            .metrics
            .catalog()
            .await
            .into_iter()
            .find(|e| e.key == "agent.tokens.total")
            .expect("still listed");
        assert!(entry.toggleable);
        assert!(!entry.enabled, "reads as disabled");
        assert!(
            svc.fact_store
                .get_spec("agent.tokens.total")
                .await
                .unwrap()
                .is_none(),
            "spec pruned on disable"
        );
        let yaml = std::fs::read_to_string(oxplow_config::config_path(dir.path())).unwrap();
        assert!(
            yaml.contains("agent.tokens.total") && yaml.contains("enabled: false"),
            "disable marker persisted; got:\n{yaml}"
        );

        // Re-enable removes the marker and re-seeds the spec from its definition.
        svc.metrics
            .set_metric_enabled("agent.tokens.total", true)
            .await
            .unwrap();
        assert!(
            svc.metrics
                .catalog()
                .await
                .iter()
                .find(|e| e.key == "agent.tokens.total")
                .unwrap()
                .enabled,
            "re-enabled"
        );
        assert!(
            svc.fact_store
                .get_spec("agent.tokens.total")
                .await
                .unwrap()
                .is_some(),
            "spec re-seeded on enable"
        );
        let yaml = std::fs::read_to_string(oxplow_config::config_path(dir.path())).unwrap();
        assert!(
            !yaml.contains("agent.tokens.total"),
            "marker cleared on re-enable; got:\n{yaml}"
        );
    }

    #[tokio::test]
    async fn set_metrics_enabled_batches_a_whole_section() {
        let (svc, dir) = fixture().await;
        svc.metrics.seed_catalog().await;
        let keys = vec![
            "agent.tokens.total".to_string(),
            "agent.tokens.input".to_string(),
            "agent.tokens.output".to_string(),
        ];
        svc.metrics.set_metrics_enabled(&keys, false).await.unwrap();

        let cat = svc.metrics.catalog().await;
        for k in &keys {
            assert!(
                !cat.iter().find(|e| &e.key == k).unwrap().enabled,
                "{k} disabled by the batch"
            );
        }
        // One config write carrying all three markers.
        let yaml = std::fs::read_to_string(oxplow_config::config_path(dir.path())).unwrap();
        assert_eq!(
            yaml.matches("enabled: false").count(),
            3,
            "one marker per key; got:\n{yaml}"
        );
    }

    #[tokio::test]
    async fn disabled_measure_closes_the_producer_collection_gate() {
        // The keystone of "stop collecting": once every metric over a measure is
        // disabled (its specs pruned), `measure_has_active_spec` is false so the
        // producer skips the write.
        let (svc, _dir) = fixture().await;
        svc.metrics.seed_catalog().await;
        assert!(
            svc.fact_store
                .measure_has_active_spec("oxplow.tokens")
                .await
                .unwrap(),
            "token specs active by default → gate open"
        );

        // Disable ALL three token metrics that source oxplow.tokens.
        for k in [
            "agent.tokens.total",
            "agent.tokens.input",
            "agent.tokens.output",
        ] {
            svc.metrics.set_metric_enabled(k, false).await.unwrap();
        }
        assert!(
            !svc.fact_store
                .measure_has_active_spec("oxplow.tokens")
                .await
                .unwrap(),
            "all consumers disabled → gate closed → producer stops collecting"
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
        assert_eq!(rel, "oxplow/gauges/acme_todo_density.star");

        // Script stub written + uses the public capability surface + emits facts.
        let script = std::fs::read_to_string(dir.path().join(&rel)).unwrap();
        assert!(script.contains("def transform(input):"), "got:\n{script}");
        assert!(
            script.contains("files(\"**/*.rs\")"),
            "glob threaded; got:\n{script}"
        );
        assert!(
            script.contains("acme.todo_density.count"),
            "emits the measure; got:\n{script}"
        );

        // The trio (measure + gauge + metric) persisted to project.yaml.
        let yaml = std::fs::read_to_string(oxplow_config::config_path(dir.path())).unwrap();
        assert!(yaml.contains("metrics:"), "metric spec; got:\n{yaml}");
        assert!(yaml.contains("gauges:"), "gauge; got:\n{yaml}");
        assert!(yaml.contains("measures:"), "measure; got:\n{yaml}");
        assert!(
            yaml.contains("acme.todo_density.count"),
            "measure key persisted; got:\n{yaml}"
        );
        assert!(
            yaml.contains("acme_todo_density.star"),
            "entryFile persisted; got:\n{yaml}"
        );

        // Metric SPEC seeded as a project-scoped gauge (T-E2).
        let spec = svc
            .fact_store
            .get_spec("acme.todo_density")
            .await
            .unwrap()
            .expect("scaffolded spec seeded");
        assert_eq!(spec.display_kind, "gauge");
        assert_eq!(spec.scope, "project");

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
    async fn global_catalog_caches_until_invalidated() {
        // tsk17: the global catalog loads from disk once, then serves the cache;
        // a file added after the first read isn't seen until invalidation.
        let (svc, _dir) = fixture().await;
        let gtmp = tempfile::tempdir().unwrap();
        let m = svc
            .metrics
            .clone()
            .with_global_dir(gtmp.path().to_path_buf());

        // First read: empty (no global measures yet) — and caches that.
        assert_eq!(m.with_global_catalog(|g| g.measures.len()), 0);

        // Write a global measure file directly (an "external" edit).
        std::fs::create_dir_all(gtmp.path().join("measures")).unwrap();
        std::fs::write(
            gtmp.path().join("measures").join("acme.yaml"),
            "measures:\n  - key: acme.thing\n",
        )
        .unwrap();

        // Still 0 — served from cache, the new file isn't re-read.
        assert_eq!(m.with_global_catalog(|g| g.measures.len()), 0, "cached");

        // After invalidation the reload picks it up.
        m.invalidate_global_catalog();
        assert_eq!(m.with_global_catalog(|g| g.measures.len()), 1, "reloaded");
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

        // Gauge script under <global>/gauges/; metric manifest under
        // <global>/metrics/; both, not the project.
        assert!(std::path::Path::new(&path).exists(), "script at {path}");
        assert!(path.ends_with("gauges/myglobal_todo.star"), "got {path}");
        let manifest =
            std::fs::read_to_string(gtmp.path().join("metrics/myglobal_todo.yaml")).unwrap();
        assert!(manifest.contains("myglobal.todo"), "got:\n{manifest}");
        // The gauge manifest names the script; the measure manifest the fact type.
        let gauge_manifest =
            std::fs::read_to_string(gtmp.path().join("gauges/myglobal_todo.yaml")).unwrap();
        assert!(
            gauge_manifest.contains("myglobal_todo.star"),
            "got:\n{gauge_manifest}"
        );
        assert!(gtmp.path().join("measures/myglobal_todo.yaml").exists());

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
        // The global gauge's script lives under <global>/gauges/, NOT the
        // project — the project dir has no such file.
        std::fs::create_dir_all(gtmp.path().join("gauges")).unwrap();
        std::fs::write(
            gtmp.path().join("gauges/g.star"),
            "def transform(input):\n    return {\"facts\": [{\"measure\": \"oxplow.ast_hit\", \"value\": float(len(files(\"**/*\"))), \"rule\": \"filecount\", \"subject\": \"tree:.\"}]}\n",
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
            effort_id: None,
            scan_kind: "delta",
        };
        let count = m.run_one_gauge(&metric, &ctx, Arc::new(files)).await;
        assert_eq!(count, 1, "global-scope script resolved + ran");
        let measure = svc
            .fact_store
            .get_measure("oxplow.ast_hit")
            .await
            .unwrap()
            .expect("oxplow.ast_hit seeded by migration");
        let facts = svc.fact_store.facts_for_measure(measure.id).await.unwrap();
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].value, 2.0, "counted both files");
    }
}
