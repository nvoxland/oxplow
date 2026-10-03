//! The metric runner (epic tsk213, P3; P7.B3): ties config-declared
//! `metrics:` entries to the substrate, and runs the **fact collectors** —
//! the collectors that record facts (`collectors:` with `facts:`, the
//! project's, an extension's, or a built-in one a `metrics: - use:`
//! enables). The `collector.triggers` pump consumer hands it the ones an
//! event triggers (`snapshot.taken`, `effort.finished`, …); `collector.sync`
//! runs one by hand; `metric.rebuild` baselines them.
//!
//! A fact collector's Starlark script runs with a [`TreeHost`] exposing the
//! snapshot's file map, so it can call `files(glob)` / `ast_query(...)`; its
//! input is `{report?, rows?, event?}` like every collector's. Each run
//! records one capture of facts and its `collector_run` +
//! `collector.synced@1`.
//!
//! Best-effort, like the other producers (`token_usage.rs` / `collection.rs`):
//! a compute/write error is logged via `tracing::warn!`, never propagated, and
//! never blocks the host path. A recorded capture is announced by the change
//! loop (`models_changed.rs`, the one maker of `MetricSamplesChanged`), not
//! here.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use oxplow_collect_plugin::{
    builtin_metrics, BuiltinMetric, CollectedFact, SandboxBudget, TreeHost,
};
use oxplow_config::collectors::{CollectorRuntime, CollectorSpec, ReportInput, Trigger};
use oxplow_config::{
    global_config_dir, load_global_dimension_entries, load_global_measure_entries,
    load_global_metric_entries, resolve_dimensions, resolve_measures, resolve_metrics,
    DimensionEntry, MeasureEntry, MetricEntry, OxplowConfig, ResolvedSpec,
};
use oxplow_db::{
    EffortStore, NewDimension, NewMeasure, NewMetricSpec, SnapshotStorage, SqliteEffortStore,
    SqliteFactStore, SqliteSnapshotStore, SqliteThreadStore,
};
use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{DomainError, EffortId, StreamId, ThreadId};
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::producer_metrics::builtin_producer_metrics;
use crate::snapshot_content::SnapshotContent;

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
const FACT_COLLECTOR_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// A gauge sweep over at least this many files is a WHOLE-TREE sweep (the baseline)
/// rather than an ordinary per-commit delta, so it gets tracked as a visible
/// background task (tsk48). A delta is a handful of files and finishes in
/// milliseconds; tracking those would just be noise.
const TREE_SWEEP_FILE_THRESHOLD: usize = 100;

/// What a gauge sweep did — how many gauges actually ran (after the idempotency
/// skip) and which failed. Returned so [`MetricsService::rebuild_baseline`]
/// can report the outcome to an MCP caller or a test instead of it vanishing into a
/// background-task label (tsk50).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SweepReport {
    pub ran: usize,
    pub failed: Vec<String>,
}

/// A collector that records facts, as the fact engine runs it (P7.B3): one
/// of the project's `collectors:`, an enabled extension's, or a built-in
/// one (`oxplow.*`) that `metrics: - use:` enabled.
#[derive(Debug, Clone, PartialEq)]
pub struct FactCollector {
    /// Its id — the producer name its captures carry.
    pub key: String,
    /// `built-in`, `project` or the extension's name.
    pub owner: String,
    pub trigger: Trigger,
    /// The measures its facts may land on; empty for a built-in (the
    /// catalog alone governs those).
    pub facts: Vec<String>,
    pub runtime: CollectorRuntime,
    /// Its script or program, relative to its owner's folder; `None` for a
    /// built-in (the script is embedded).
    pub entry: Option<String>,
    pub report: Option<ReportInput>,
    /// Read-only SQL handed over as `input.rows`, the trigger's anchors bound.
    pub input: Option<String>,
    /// With an `on:` trigger: the pump consumers that must have handled the
    /// event first.
    pub after: Vec<String>,
    /// It reads the whole tree as of the snapshot on every run, so its
    /// capture restates every file (a built-in whole-tree scan).
    pub whole_tree: bool,
}

impl FactCollector {
    /// The fact collector `spec` declares, or `None` when it writes
    /// entities instead.
    pub fn from_spec(owner: &str, spec: &CollectorSpec) -> Option<Self> {
        (!spec.facts.is_empty()).then(|| FactCollector {
            key: spec.id.clone(),
            owner: owner.to_string(),
            trigger: spec.trigger.clone(),
            facts: spec.facts.clone(),
            runtime: spec.runtime,
            entry: spec.entry.clone(),
            report: spec.report.clone(),
            input: spec.input.clone(),
            after: spec.after.clone(),
            whole_tree: false,
        })
    }

    /// A bundled metric's collector, on the trigger the catalog gives it.
    fn builtin(m: &BuiltinMetric) -> Self {
        FactCollector {
            key: m.key.to_string(),
            owner: oxplow_config::collectors::BUILT_IN.to_string(),
            trigger: Trigger::On {
                events: m.on.iter().map(|e| e.to_string()).collect(),
                filter: m
                    .filter
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            },
            facts: Vec::new(),
            runtime: CollectorRuntime::Starlark,
            entry: None,
            report: None,
            input: None,
            after: Vec::new(),
            whole_tree: m.whole_tree,
        }
    }

    fn is_builtin(&self) -> bool {
        self.owner == oxplow_config::collectors::BUILT_IN
    }

    /// Whether an event of `event_type` triggers it.
    pub fn runs_on(&self, event_type: &str) -> bool {
        matches!(&self.trigger, Trigger::On { events, .. } if events.iter().any(|e| e == event_type))
    }

    /// The scope its captures record (the metric catalog's vocabulary).
    fn scope(&self) -> String {
        match self.owner.as_str() {
            o @ (oxplow_config::collectors::BUILT_IN | oxplow_config::collectors::PROJECT) => {
                o.to_string()
            }
            ext => oxplow_config::extension_scope(ext),
        }
    }
}

/// Whether the `snapshot.taken` `event` recorded files (it isn't an
/// unchanged take).
pub(crate) fn take_recorded(event: &oxplow_domain::StoredEvent) -> bool {
    let payload = &event.envelope.payload;
    !payload["unchanged"].as_bool().unwrap_or(false)
        && payload["file_count"].as_u64().unwrap_or(0) > 0
}

/// A built-in's trigger as the catalog shows it: `on snapshot.taken`,
/// with its payload filter when it has one.
fn trigger_label(m: &BuiltinMetric) -> String {
    let filter: Vec<String> = m.filter.iter().map(|(k, v)| format!("{k}: {v}")).collect();
    match filter.is_empty() {
        true => format!("on {}", m.on.join(", ")),
        false => format!("on {} where {}", m.on.join(", "), filter.join(", ")),
    }
}

/// The event that runs snapshot-triggered fact collectors.
const SNAPSHOT_TAKEN: &str = "snapshot.taken";
/// The event that runs effort-triggered fact collectors.
const EFFORT_FINISHED: &str = "effort.finished";

/// How one fact collector's run went.
#[derive(Debug, Clone, PartialEq)]
pub enum FactRun {
    /// It recorded this many facts.
    Recorded(usize),
    /// It failed (recorded as a failed capture).
    Failed(String),
    /// It had already run over this snapshot at its current logic.
    Skipped,
}

/// Runs config-declared metrics into the substrate. Cheap to clone (a handle of
/// leaf `Arc`s) — deliberately NOT holding `Arc<Services>`, to avoid a cycle.
/// The global-scope catalog blocks parsed from `<global_dir>/{metrics,
/// measures,dimensions}/*.yaml`. Cached so the hot read paths
/// (`resolved_specs`, run on every snapshot event) don't
/// re-read + re-parse these files each time (tsk17). Project config stays read
/// fresh from the in-memory `RwLock` — only the *disk* loads are cached.
#[derive(Default)]
struct GlobalCatalog {
    metrics: Vec<MetricEntry>,
    measures: Vec<MeasureEntry>,
    dimensions: Vec<DimensionEntry>,
}

#[derive(Clone)]
pub struct MetricsService {
    snapshot_store: Arc<SqliteSnapshotStore>,
    thread_store: Arc<SqliteThreadStore>,
    effort_store: Arc<SqliteEffortStore>,
    content: SnapshotContent,
    vcs: Arc<dyn oxplow_domain::vcs::Vcs>,
    config: Arc<RwLock<OxplowConfig>>,
    project_dir: PathBuf,
    /// This machine's program approvals (`exec_consent`).
    approvals: Arc<crate::exec_consent::ApprovalStore>,
    /// Override for the global config dir (the parent of `metrics/`). `None` →
    /// the platform `global_config_dir()`. A field (not the free fn) so tests
    /// can point it at a tempdir without racing on a process-global env var.
    global_dir: Option<PathBuf>,
    /// The fact substrate, for seeding config-declared `measures:`/`dimensions:`
    /// into the catalog (epic tsk12, E). `None` in test fixtures that don't
    /// exercise catalog seeding; wired at boot via [`Self::with_fact_store`].
    fact_store: Option<Arc<SqliteFactStore>>,
    /// Wired at boot via [`Self::with_snapshot_captures`]; `None` in tests
    /// that never rebuild a baseline.
    snapshot_captures: Option<crate::snapshot_capture_registry::SnapshotCaptureRegistry>,
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
    /// The loaded-extensions cache shared with `Services`; a fresh one
    /// for a bare `MetricsService` in tests.
    extensions_cache: Arc<crate::extension_catalog::ExtensionCatalog>,
    /// Per state entity metric: when it was last captured and the value
    /// (tsk322), for the throttle and the unchanged-value skip.
    entity_captures: Arc<std::sync::Mutex<HashMap<String, (std::time::Instant, f64)>>>,
    /// Where a fact collector's run is recorded (`collector_run` +
    /// `collector.synced@1`) and its `input` read. `None` in tests that don't
    /// wire it: the run still records its capture.
    run_log: Option<crate::collector_runner::RunLog>,
}

/// How often a state entity metric may be re-captured.
const ENTITY_CAPTURE_EVERY: std::time::Duration = std::time::Duration::from_secs(600);

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
struct CollectorRunContext {
    stream_val: i64,
    thread_id: Option<i64>,
    trigger: &'static str,
    snapshot_id: Option<i64>,
    closest_vcs_rev: Option<String>,
    vcs_rev_exact: bool,
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
    /// The event that triggered the run (`on:`), when one did: the script's
    /// `input.event`, its anchors' `input` parameters, and the cause of the
    /// run's `collector.synced@1`.
    event: Option<Arc<oxplow_domain::StoredEvent>>,
    /// Who ran it, as an event source (`collector.synced@1`'s): the
    /// system, unless someone ran it by hand.
    source: String,
}

/// Measures, metrics, fact collectors and dimensions from enabled
/// extensions, per extension.
#[derive(Default)]
struct ExtensionCatalog {
    measures: Vec<oxplow_config::ExtensionLayer<oxplow_config::MeasureEntry>>,
    metrics: Vec<oxplow_config::ExtensionLayer<oxplow_config::MetricEntry>>,
    collectors: Vec<FactCollector>,
    dimensions: Vec<oxplow_config::ExtensionLayer<oxplow_config::DimensionEntry>>,
}

impl MetricsService {
    /// The branch the project checkout has checked out (`None` when
    /// detached or unreadable).
    async fn current_branch(&self) -> Option<String> {
        self.vcs
            .head(&self.project_dir)
            .await
            .ok()
            .and_then(|h| h.branch)
    }

    pub fn new(
        snapshot_store: Arc<SqliteSnapshotStore>,
        thread_store: Arc<SqliteThreadStore>,
        effort_store: Arc<SqliteEffortStore>,
        content: SnapshotContent,
        vcs: Arc<dyn oxplow_domain::vcs::Vcs>,
        config: Arc<RwLock<OxplowConfig>>,
        project_dir: PathBuf,
    ) -> Self {
        Self {
            snapshot_store,
            thread_store,
            effort_store,
            content,
            vcs,
            config,
            project_dir,
            approvals: Arc::new(crate::exec_consent::ApprovalStore::disabled()),
            global_dir: None,
            fact_store: None,
            background_tasks: None,
            global_catalog: Arc::new(std::sync::Mutex::new(None)),
            snapshot_captures: None,
            extensions_cache: Arc::new(crate::extension_catalog::ExtensionCatalog::new()),
            entity_captures: Arc::new(std::sync::Mutex::new(HashMap::new())),
            run_log: None,
        }
    }

    /// Record fact collectors' runs and read their `input` through `log`.
    pub fn with_run_log(mut self, log: crate::collector_runner::RunLog) -> Self {
        self.run_log = Some(log);
        self
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

    /// The program approvals a project's exec collectors are checked against.
    pub fn with_approvals(mut self, approvals: Arc<crate::exec_consent::ApprovalStore>) -> Self {
        self.approvals = approvals;
        self
    }

    pub fn with_fact_store(mut self, fact_store: Arc<SqliteFactStore>) -> Self {
        self.fact_store = Some(fact_store);
        self
    }

    /// The streams' snapshot captures, which a baseline rebuild drains
    /// ([`Self::rebuild_baseline`]).
    pub fn with_snapshot_captures(
        mut self,
        captures: crate::snapshot_capture_registry::SnapshotCaptureRegistry,
    ) -> Self {
        self.snapshot_captures = Some(captures);
        self
    }

    /// The effective global config dir (the field override, else the platform
    /// `global_config_dir()`); `metrics/` hangs under it.
    fn effective_global_dir(&self) -> Option<PathBuf> {
        self.global_dir.clone().or_else(global_config_dir)
    }

    /// Share the loaded-extensions cache with the rest of `Services`.
    pub fn with_extension_catalog(
        mut self,
        catalog: Arc<crate::extension_catalog::ExtensionCatalog>,
    ) -> Self {
        self.extensions_cache = catalog;
        self
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
                    measures: load_global_measure_entries(&d),
                    dimensions: load_global_dimension_entries(&d),
                },
                None => GlobalCatalog::default(),
            });
        }
        f(guard.as_ref().expect("populated above"))
    }

    /// Drop the cached global catalog so the next read reloads it. Called on
    /// every `ConfigChanged`.
    fn invalidate_global_catalog(&self) {
        *self
            .global_catalog
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = None;
    }

    /// What enabled extensions contribute to the metric catalog, per
    /// extension (read from the project's primary worktree).
    fn extension_catalog(&self) -> ExtensionCatalog {
        let mut out = ExtensionCatalog::default();
        for e in self.extensions_cache.get(&self.project_dir).iter().cloned() {
            if !e.enabled {
                continue;
            }
            if !e.measures.is_empty() {
                out.measures.push((e.name.clone(), e.measures));
            }
            if !e.metrics.is_empty() {
                out.metrics.push((e.name.clone(), e.metrics));
            }
            out.collectors.extend(
                e.collectors
                    .iter()
                    .filter_map(|c| FactCollector::from_spec(&e.name, c)),
            );
            if !e.dimensions.is_empty() {
                out.dimensions.push((e.name.clone(), e.dimensions));
            }
        }
        out
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
        let ext = self.extension_catalog();
        resolve_metrics(&builtin, &global, &ext.metrics, &project)
    }

    /// `collectors` without the ones failures disabled (P7.C2) and the ones
    /// that already ran for `event` (a redelivered event runs nothing
    /// again).
    async fn runnable(
        &self,
        collectors: Vec<FactCollector>,
        event: Option<&oxplow_domain::StoredEvent>,
    ) -> Vec<FactCollector> {
        let Some(log) = self.run_log.as_ref() else {
            return collectors;
        };
        let health = log.health();
        let mut out = Vec::new();
        for c in collectors {
            let key = crate::collector_runner::plugin_key(&c.owner, &c.key);
            // Failures disabled it (P7.C2): it waits for a person.
            if !matches!(health.disabled_reason(&key).await, Ok(None)) {
                continue;
            }
            if let Some(event) = event {
                if log.ran_for(&c.owner, &c.key, event.seq).await {
                    continue;
                }
            }
            out.push(c);
        }
        out
    }

    /// The fact collectors this project runs (P7.B3): the project's own
    /// (`collectors:` with `facts:`), enabled extensions', and the built-in
    /// ones whose metric `metrics: - use:` enables (and no marker disables).
    /// An id two owners declare runs once — the project's over an
    /// extension's over a built-in — with a warning.
    pub fn fact_collectors(&self) -> Vec<FactCollector> {
        let enabled: std::collections::HashSet<String> = self
            .resolved_specs()
            .into_iter()
            .filter(|s| s.scope == "built-in" && s.enabled)
            .map(|s| s.key)
            .collect();
        let builtin: Vec<FactCollector> = builtin_metrics()
            .iter()
            .filter(|m| enabled.contains(m.key))
            .map(FactCollector::builtin)
            .collect();
        let project: Vec<FactCollector> = self
            .config
            .read()
            .map(|c| {
                c.collectors
                    .iter()
                    .filter_map(|s| FactCollector::from_spec(oxplow_config::collectors::PROJECT, s))
                    .collect()
            })
            .unwrap_or_default();
        let mut out: Vec<FactCollector> = Vec::new();
        for c in builtin
            .into_iter()
            .chain(self.extension_catalog().collectors)
            .chain(project)
        {
            match out.iter().position(|o| o.key == c.key) {
                Some(i) => {
                    tracing::warn!(id = %c.key, kept = %c.owner, dropped = %out[i].owner,
                        "two owners declare this collector id; the later one runs");
                    out[i] = c;
                }
                None => out.push(c),
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

        let ext = self.extension_catalog();
        let layer = crate::sql_gateway::SqlGateway::new(facts.database());
        let mut m = 0;
        for rm in resolve_measures(&global_measures, &ext.measures, &project_measures) {
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
        let resolved_dims = resolve_dimensions(&global_dims, &ext.dimensions, &project_dims);
        // A disabled or removed extension's dimensions leave the catalog.
        let keep_ext_dims: Vec<String> = resolved_dims
            .iter()
            .filter(|d| oxplow_config::scope_extension(&d.scope).is_some())
            .map(|d| d.key.clone())
            .collect();
        if let Err(e) = facts
            .delete_extension_dimensions_not_in(keep_ext_dims)
            .await
        {
            tracing::warn!(error = %e, "seed: extension dimension reconciliation failed");
        }
        for rd in resolved_dims {
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
                entity_json: rd
                    .entity
                    .as_ref()
                    .and_then(|e| serde_json::to_string(e).ok()),
            };
            if seed_dimension(facts, &layer, nd).await {
                d += 1;
            }
        }
        // Built-ins aren't config-declared, so they don't count toward `d`.
        for nd in builtin_entity_dimensions() {
            seed_dimension(facts, &layer, nd).await;
        }
        // Entity dimensions by the view they slice, for entity metrics'
        // sliceable dims.
        let entity_dims: Vec<(String, String)> = facts
            .list_dimensions()
            .await
            .unwrap_or_default()
            .iter()
            .filter_map(|d| {
                crate::entity_metrics::entity_dim_of(d).map(|e| (e.view, d.key.clone()))
            })
            .collect();
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
        // un-`use:`d — its collector simply doesn't RUN (gated in `fact_collectors`) —
        // so a disable is only ever an explicit marker.
        // A spec whose aggregation the engine cannot compute must not seed
        // (tsk108): the V44 CHECK reserves `p95`/`count_distinct` in the
        // schema vocabulary and config doesn't validate the string, but an
        // unimplemented aggregation would open the COLLECTION gate
        // (`measure_has_active_spec`) for a metric that can never render —
        // data gathered for nothing, forever. Pruned like a disabled entry
        // (implementing the aggregation later re-seeds it cleanly). Formula
        // specs carry no aggregation of their own to validate.
        let computable = |spec: &oxplow_db::NewMetricSpec| {
            let ok = spec.source_measure.is_none()
                || crate::metric_engine::Aggregation::parse(&spec.aggregation).is_some();
            if !ok {
                tracing::warn!(
                    key = %spec.key, aggregation = %spec.aggregation,
                    "spec uses an aggregation the engine can't compute (reserved in the \
                     schema); pruning it so collection doesn't run for a metric that can \
                     never render"
                );
            }
            ok
        };
        for mut spec in builtin_metric_specs()
            .into_iter()
            .chain(builtin_ast_specs())
            .chain(crate::producer_metrics::builtin_producer_specs())
            .chain(builtin_entity_specs())
        {
            let key = spec.key.clone();
            let res = if config_state(&key) != Some(false)
                && computable(&spec)
                && prepare_entity_spec(facts, &layer, &mut spec, &entity_dims).await
            {
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
        let resolved = self.resolved_specs();
        for s in &resolved {
            let mut spec = spec_to_new_spec(s);
            let res = if s.enabled
                && computable(&spec)
                && prepare_entity_spec(facts, &layer, &mut spec, &entity_dims).await
            {
                facts.upsert_spec(spec).await.map(|_| ())
            } else {
                facts.delete_spec(&s.key).await
            };
            if let Err(e) = res {
                tracing::warn!(key = %s.key, error = %e, "failed to reconcile config metric spec");
            }
        }
        // RECONCILE REMOVALS (tsk61): a project metric/measure deleted from
        // `.oxplow/project.yaml` entirely (not merely `enabled: false`) used to
        // leave zombie catalog rows behind — four forever-blank gauges sat in
        // the catalog for a week after their gauges were replaced. Project
        // scope only: the declared config is the truth for exactly its scope.
        let keep_specs: Vec<String> = resolved
            .iter()
            .filter(|s| s.scope == "project")
            .map(|s| s.key.clone())
            .collect();
        match facts.delete_project_specs_not_in(keep_specs).await {
            Ok(n) if n > 0 => {
                tracing::info!(pruned = n, "seed: dropped undeclared project metric specs");
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "seed: project spec reconciliation failed"),
        }
        // Same for extension-declared specs: a disabled or removed extension's
        // metrics leave the catalog (their measures and facts stay).
        let keep_extension_specs: Vec<String> = resolved
            .iter()
            .filter(|s| oxplow_config::scope_extension(&s.scope).is_some())
            .map(|s| s.key.clone())
            .collect();
        match facts
            .delete_extension_specs_not_in(keep_extension_specs)
            .await
        {
            Ok(n) if n > 0 => {
                tracing::info!(
                    pruned = n,
                    "seed: dropped specs of disabled or removed extensions"
                );
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "seed: extension spec reconciliation failed"),
        }
        let keep_measures: Vec<String> =
            resolve_measures(&global_measures, &ext.measures, &project_measures)
                .into_iter()
                .filter(|rm| rm.scope == "project")
                .map(|rm| rm.key)
                // A project state entity metric's synthesized measure.
                .chain(
                    resolved
                        .iter()
                        .filter(|s| s.scope == "project")
                        .filter(|s| s.entity.as_ref().is_some_and(|e| e.time.is_none()))
                        .map(|s| s.key.clone()),
                )
                .collect();
        match facts.delete_project_measures_not_in(keep_measures).await {
            Ok(n) if n > 0 => {
                tracing::info!(pruned = n, "seed: dropped undeclared project measures");
            }
            Ok(_) => {}
            Err(e) => tracing::warn!(error = %e, "seed: project measure reconciliation failed"),
        }
        if let Err(e) = self.publish_catalog(facts).await {
            tracing::warn!(error = %e, "seed: publishing the metric catalog failed");
        }
        (m, d)
    }

    /// Write the resolved catalog to `metric_catalog` (`v_metric_catalog`,
    /// P4.7) — the whole of it, replacing what was there.
    async fn publish_catalog(&self, facts: &SqliteFactStore) -> Result<(), DomainError> {
        let entries = self.catalog().await;
        facts
            .database()
            .transaction(move |tx| {
                tx.execute("DELETE FROM metric_catalog", [])
                    .map_err(oxplow_db::map_sql_err)?;
                for e in &entries {
                    tx.execute(
                        "INSERT INTO metric_catalog
                           (key, title, kind, language, scope, enabled, target, trigger, toggleable, category)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                        rusqlite::params![
                            e.key, e.title, e.kind, e.language, e.scope, e.enabled, e.target,
                            e.trigger, e.toggleable, e.category
                        ],
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                }
                Ok(())
            })
            .await
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
                trigger: trigger_label(&b),
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
        // Built-in entity metrics (tsk322): default-ON like the producers, and
        // listed from code so a disabled one (its spec pruned) can come back.
        for e in builtin_entity_specs() {
            if seen.insert(e.key.clone()) {
                out.push(MetricCatalogEntry {
                    enabled: config_state(&e.key) != Some(false),
                    key: e.key,
                    title: e.title,
                    kind: e.display_kind,
                    language: None,
                    scope: "built-in".to_string(),
                    target: e.target,
                    trigger: "auto".to_string(),
                    toggleable: true,
                    category: e.category,
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
    pub(crate) fn apply_metric_enabled(
        &self,
        metrics: &mut Vec<MetricEntry>,
        key: &str,
        enabled: bool,
    ) {
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

    /// A new collector-backed metric, as a template (epic tsk12, E; tsk391):
    /// a starter Starlark collector script plus the **trio** that wires it
    /// up — a `measures:` entry (`<key>.count`, the fact type it records), a
    /// `collectors:` entry (`<key>`, the producer) and a `metrics:` spec
    /// (`<key>`, a `sum` over that measure) as a `.oxplow/project.yaml`
    /// snippet. Writes nothing: the agent writes the script and merges the
    /// snippet with its own file tools, so the write guard, filing and its
    /// worktree apply (the extensions "no scaffold tools" rule). A config
    /// change reseeds the catalog once they land.
    pub fn metric_scaffold(
        &self,
        key: &str,
        title: Option<String>,
        language: Option<String>,
        glob: Option<String>,
    ) -> Result<MetricScaffold, String> {
        let key = key.trim();
        if key.is_empty() || !key.contains('.') {
            return Err("key must be namespaced, e.g. acme.my_metric".to_string());
        }
        if key.starts_with("oxplow.") {
            return Err("`oxplow.` is reserved for built-in metrics".to_string());
        }
        {
            let cfg = self
                .config
                .read()
                .map_err(|_| "config lock poisoned".to_string())?;
            if cfg.metrics.iter().any(|e| e.key.as_deref() == Some(key))
                || cfg.collectors.iter().any(|c| c.id == key)
            {
                return Err(format!(
                    "metric `{key}` already exists in .oxplow/project.yaml"
                ));
            }
        }
        let glob = glob
            .filter(|g| !g.is_empty())
            .unwrap_or_else(|| "**/*".into());
        let language = language.filter(|l| !l.is_empty());
        let slug = slugify(key);
        let measure_key = format!("{key}.count");
        let title = title.filter(|t| !t.is_empty());
        let script = starter_collector_script(key, &measure_key, &glob, language.as_deref());
        let script_path = format!("oxplow/collectors/{slug}.star");

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
        // The collector (producer) — records `<key>.count` facts on every
        // snapshot.
        let mut collector = serde_yaml::Mapping::new();
        collector.insert("id".into(), key.into());
        if let Some(t) = &title {
            collector.insert("doc".into(), t.as_str().into());
        }
        collector.insert("runtime".into(), "starlark".into());
        collector.insert("entry".into(), script_path.as_str().into());
        let mut trigger = serde_yaml::Mapping::new();
        trigger.insert(
            "on".into(),
            serde_yaml::Value::Sequence(vec![SNAPSHOT_TAKEN.into()]),
        );
        collector.insert("trigger".into(), serde_yaml::Value::Mapping(trigger));
        collector.insert(
            "facts".into(),
            serde_yaml::Value::Sequence(vec![measure_key.as_str().into()]),
        );
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

        Ok(MetricScaffold {
            key: key.to_string(),
            project_yaml: oxplow_config::entries_yaml(
                &[measure],
                &[serde_yaml::Value::Mapping(collector)],
                &[metric],
            ),
            script_path,
            script,
        })
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
    /// [`Self::collectors_needing_baseline`] on the next snapshot that lands.
    pub async fn needs_tree_baseline(&self, stream_id: i64) -> bool {
        !self.collectors_needing_baseline(stream_id).await.is_empty()
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
    /// the last baseline" (the old `collector_is_stale` criterion — a full capture at
    /// stale logic carries the old fingerprint and doesn't match).
    ///
    /// This set is the pending-baseline QUEUE: the on-snapshot sweep drains it by
    /// running these gauges `full` over the next snapshot that lands, so a newly
    /// added or edited gauge baselines on the next ordinary snapshot — no
    /// fabricated full-tree snapshot (which used to pollute effort attribution).
    pub async fn collectors_needing_baseline(&self, stream_id: i64) -> Vec<String> {
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
        for gauge in self.fact_collectors() {
            if !gauge.runs_on(SNAPSHOT_TAKEN) {
                continue;
            }
            // Fingerprint-scoped when the script is hashable; any-version
            // otherwise (an unfingerprintable collector can't detect
            // staleness, so one full capture ever is the best we can require).
            let fp = collector_fingerprint(&gauge, &self.project_dir);
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
    pub async fn collector_is_stale(&self, gauge: &FactCollector, stream_id: i64) -> bool {
        let Some(facts) = self.fact_store.as_ref() else {
            return false;
        };
        let Some(current) = collector_fingerprint(gauge, &self.project_dir) else {
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

    /// Bring every `per-path` metric up to date over the WHOLE tree (tsk50).
    ///
    /// A baseline no longer fabricates a full-tree snapshot (tsk71 — that
    /// snapshot polluted effort file-attribution). Instead it drains any
    /// pending edits into an ORDINARY snapshot (authored work, correctly
    /// attributed), anchors on the latest snapshot, and runs the un-baselined
    /// gauges `scan_kind = 'full'` over the RECONSTRUCTED tree as-of it — the
    /// per-path fold reads a full capture's scanned set via `tree_at`
    /// semantics, so provenance stays on a real snapshot and no snapshot is
    /// invented.
    ///
    /// This is deliberately callable, not buried in boot: it's the one entry point
    /// boot, the `metric.rebuild` command, and the end-to-end test all share, so
    /// the boot baseline path is finally exercisable without a process restart —
    /// four metrics bugs in a row (tsk47/48/49) were caught only by restarting.
    ///
    /// `force` treats every gauge as needing a baseline (the command's escape hatch).
    /// The kind-scoped idempotency guard in the sweep means a repeat over the
    /// same unchanged snapshot won't re-scan.
    pub async fn rebuild_baseline(&self, force: bool) -> Result<BaselineReport, String> {
        let Some(captures) = &self.snapshot_captures else {
            return Err("no snapshot captures wired".into());
        };
        let Some(fact_store) = &self.fact_store else {
            return Err("no fact store wired".into());
        };
        let stream_val = captures
            .primary()
            .map(|c| c.stream_id().value())
            .unwrap_or(1);

        if !force {
            let pending = self.collectors_needing_baseline(stream_val).await;
            if pending.is_empty() {
                // Nothing to baseline — but still sweep out captures an
                // EARLIER baseline made dead weight (tsk75): the post-sweep
                // prune only fires when a full phase runs, so history from
                // before the prune existed is collected here, once per boot.
                // Idempotent and cheap when there's nothing to drop.
                match fact_store.prune_dominated_tree_captures(stream_val).await {
                    Ok(n) if n > 0 => {
                        tracing::info!(
                            pruned = n,
                            "metrics: dropped baseline-dominated tree captures at boot"
                        );
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!(error = %e, "metrics: boot prune failed");
                    }
                }
                return Ok(BaselineReport {
                    ran: false,
                    ..Default::default()
                });
            }
        }

        let Some(capture) = captures.primary() else {
            return Err("no primary snapshot capture registered".into());
        };
        // Make sure the tree we anchor on is current: wait for the startup
        // sweep, run the ordinary stat-diff (which marks only files whose
        // content actually changed since their last capture — NOT the old
        // pretend-everything-is-dirty full-tree enqueue), and drain the dirty
        // set into a normal snapshot. Those rows are real authored edits the
        // fs-watch would capture anyway — attribution is correct-by-
        // construction. `None` = nothing changed; fall back to the latest
        // existing snapshot.
        capture.await_initial_ready().await;
        capture
            .enqueue_startup_diff()
            .await
            .map_err(|e| e.to_string())?;
        let drained = capture
            .request_snapshot(oxplow_domain::snapshot::SnapshotTrigger::Manual)
            .await
            .map_err(|e| e.to_string())?;
        let snapshot_id = match drained {
            Some(id) => Some(id),
            None => self
                .snapshot_store
                .latest_snapshot_id_for_stream(oxplow_domain::StreamId::new(stream_val))
                .await
                .map_err(|e| e.to_string())?,
        };
        let Some(snapshot_id) = snapshot_id else {
            // Fresh project with no snapshot at all — nothing to baseline on.
            return Ok(BaselineReport {
                ran: false,
                ..Default::default()
            });
        };

        let sweep = self
            .run_snapshot_collectors(
                oxplow_domain::StreamId::new(stream_val),
                snapshot_id,
                force,
                None,
            )
            .await;
        Ok(BaselineReport {
            ran: true,
            snapshot_id: Some(snapshot_id),
            collectors_run: sweep.ran,
            failed: sweep.failed,
        })
    }

    /// Reseed the catalog from scratch — the global catalog dropped, the
    /// config's and the extensions' declarations seeded — and re-capture
    /// every state entity metric. What a config change (`config.metrics`)
    /// and an extension change (the catalog's signal) run.
    pub async fn reseed(&self) {
        self.invalidate_global_catalog();
        self.seed_catalog().await;
        self.capture_entity_states(true).await;
    }

    /// Seed once, then reseed whenever the primary worktree's extensions
    /// may have changed (`changes`, the extension catalog's signal: their
    /// measures, metrics and collectors). A config change reseeds through
    /// `config.metrics`; rows moving re-capture state metrics through
    /// `metrics.entity_states`; fact collectors run from
    /// `collector.triggers`. Spawned at boot (`boot.rs`).
    pub async fn run(self, mut changes: tokio::sync::broadcast::Receiver<()>) {
        self.seed_catalog().await;
        self.capture_entity_states(true).await;
        // Lagging only means it missed some: one pass covers them.
        while let Ok(()) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) =
            changes.recv().await
        {
            self.reseed().await;
        }
    }

    /// Capture each state entity metric's current value as a fact on its
    /// synthesized measure (producer `entity-metric`, tsk322), so it has
    /// history. At most once per [`ENTITY_CAPTURE_EVERY`] per metric unless
    /// `force`d, and never when the value hasn't moved since the last capture
    /// (a level carries forward). Recorded against the primary stream.
    /// Returns how many were captured.
    pub async fn capture_entity_states(&self, force: bool) -> usize {
        let Some(facts) = self.fact_store.as_ref() else {
            return 0;
        };
        let Ok(specs) = facts.list_specs().await else {
            return 0;
        };
        let layer = crate::sql_gateway::SqlGateway::new(facts.database());
        let mut stream: Option<i64> = None;
        let mut captured = 0;
        for spec in specs {
            let Some(entity) = crate::entity_metrics::entity_of(&spec) else {
                continue;
            };
            if entity.time.is_some() {
                continue;
            }
            let last = self
                .entity_captures
                .lock()
                .ok()
                .and_then(|m| m.get(&spec.key).copied());
            if !force && last.is_some_and(|(at, _)| at.elapsed() < ENTITY_CAPTURE_EVERY) {
                continue;
            }
            let value = match crate::entity_metrics::current(&layer, &entity, None).await {
                Ok(rows) => rows.first().and_then(|r| r.value).unwrap_or(0.0),
                Err(e) => {
                    tracing::warn!(key = %spec.key, error = %e, "entity metric: read failed");
                    continue;
                }
            };
            let now = std::time::Instant::now();
            let Ok(Some(measure)) = facts.get_measure(&spec.key).await else {
                continue;
            };
            // Nothing in memory (a fresh process): the latest stored capture
            // is what the level currently reads.
            let previous = match last {
                Some((_, v)) => Some(v),
                None => facts
                    .facts_for_measure(measure.id)
                    .await
                    .ok()
                    .and_then(|fs| fs.into_iter().max_by_key(|f| f.capture_id))
                    .map(|f| f.value),
            };
            if previous == Some(value) {
                if let Ok(mut m) = self.entity_captures.lock() {
                    m.insert(spec.key.clone(), (now, value));
                }
                continue;
            }
            if stream.is_none() {
                stream = primary_stream(&layer).await;
            }
            let Some(stream_id) = stream else {
                return captured;
            };
            let capture = oxplow_db::NewMetricCapture::done(stream_id, "entity-metric", "entity");
            if let Err(e) = facts
                .record_facts(capture, vec![oxplow_db::NewFact::new(measure.id, value)])
                .await
            {
                tracing::warn!(key = %spec.key, error = %e, "entity metric: record failed");
                continue;
            }
            if let Ok(mut m) = self.entity_captures.lock() {
                m.insert(spec.key.clone(), (now, value));
            }
            captured += 1;
        }
        captured
    }

    /// Run every enabled `on-snapshot` gauge against the just-captured snapshot.
    /// `pub(crate)` so [`MetricsService::rebuild_baseline`] can drive it
    /// directly (and thus test the boot path end to end, tsk50).
    ///
    /// Two-phase (tsk71): gauges already baselined run a `delta` scan over the
    /// snapshot's own file rows (the cheap incremental rescan); gauges in the
    /// pending-baseline queue ([`Self::collectors_needing_baseline`]) run a `full`
    /// scan over the RECONSTRUCTED tree as-of this snapshot and record
    /// `scan_kind = 'full'` captures anchored to it. So a baseline needs no
    /// fabricated full-tree snapshot — it piggybacks on whatever ordinary
    /// snapshot lands next.
    /// [`Self::run_snapshot_collectors`] with a `force_full` override: treat EVERY
    /// on-snapshot gauge as needing a baseline (the `metric.rebuild { force }`
    /// escape hatch). The per-snapshot idempotency guard still applies, so a
    /// repeated force over the same unchanged snapshot doesn't re-scan.
    pub(crate) async fn run_snapshot_collectors(
        &self,
        stream_id: StreamId,
        snapshot_id: i64,
        force_full: bool,
        event: Option<Arc<oxplow_domain::StoredEvent>>,
    ) -> SweepReport {
        // A take that recorded no files (a ref move on a clean tree) has
        // no delta: only the whole-tree collectors have the moved revision
        // to restate.
        let recorded = event.as_ref().is_none_or(|e| take_recorded(e));
        let gauges: Vec<FactCollector> = self
            .fact_collectors()
            .into_iter()
            .filter(|g| g.runs_on(SNAPSHOT_TAKEN))
            .filter(|g| recorded || g.whole_tree)
            .filter(|g| {
                event
                    .as_ref()
                    .is_none_or(|e| crate::collector_triggers::matches_where(&g.trigger, e))
            })
            .collect();
        let gauges = self.runnable(gauges, event.as_deref()).await;
        if gauges.is_empty() {
            return SweepReport::default();
        }
        let needing: std::collections::HashSet<String> = if force_full {
            gauges.iter().map(|g| g.key.clone()).collect()
        } else {
            self.collectors_needing_baseline(stream_id.value())
                .await
                .into_iter()
                .collect()
        };
        let (full_gauges, delta_gauges): (Vec<FactCollector>, Vec<FactCollector>) = gauges
            .into_iter()
            .partition(|g| g.whole_tree || needing.contains(&g.key));

        let mut report = SweepReport::default();
        if !delta_gauges.is_empty() {
            // The snapshot's own rows — the incremental rescan corpus.
            let files = Arc::new(self.build_file_map(snapshot_id).await);
            let mut ctx = self
                .snapshot_context(stream_id.value(), None, SNAPSHOT_TAKEN, snapshot_id)
                .await;
            ctx.event = event.clone();
            let r = self.run_collector_sweep(&delta_gauges, &ctx, files).await;
            report.ran += r.ran;
            report.failed.extend(r.failed);
        }
        if !full_gauges.is_empty() {
            // The reconstructed whole tree as-of this snapshot — the baseline
            // corpus. Built only when something actually needs baselining.
            let files = Arc::new(self.build_full_file_map(snapshot_id).await);
            let mut ctx = self
                .snapshot_context(stream_id.value(), None, SNAPSHOT_TAKEN, snapshot_id)
                .await;
            ctx.scan_kind = "full";
            ctx.event = event.clone();
            // Whole-tree collectors restate the tree on every qualifying
            // take; only the `needing` ones are baselines (tsk709).
            let baselined = full_gauges.iter().any(|g| needing.contains(&g.key));
            let r = self.run_collector_sweep(&full_gauges, &ctx, files).await;
            let full_ok = r.failed.is_empty();
            report.ran += r.ran;
            report.failed.extend(r.failed);
            // A fresh baseline makes every older effort-less tree capture dead
            // weight (tsk75 — their facts were ~69% of the table and every
            // full-history read paid for them). Prune only on a clean baseline
            // sweep: a failed gauge wrote no baseline, so its history must
            // survive, and a whole-tree restate alone baselines nothing.
            if full_ok && baselined {
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
    /// sweep. Split out from [`Self::run_snapshot_collectors`] so the tracking is
    /// exercisable without standing up real snapshot blobs.
    async fn run_collector_sweep(
        &self,
        gauges: &[FactCollector],
        ctx: &CollectorRunContext,
        files: Arc<HashMap<String, String>>,
    ) -> SweepReport {
        // Idempotency: skip a gauge that already has a `done` capture for THIS
        // snapshot at its current fingerprint (tsk50). Otherwise a re-delivered
        // snapshot event — or the direct baseline run PLUS the event loop reacting to
        // the same snapshot — would tree-sitter-parse the whole tree twice (minutes of
        // CPU). The manual `collector.sync` path doesn't come through here, so an explicit
        // "run now" still runs.
        let mut to_run: Vec<&FactCollector> = Vec::new();
        for g in gauges {
            let already = match (ctx.snapshot_id, self.fact_store.as_ref()) {
                (Some(snap), Some(facts)) => {
                    let fp = collector_fingerprint(g, &self.project_dir);
                    facts
                        .collector_done_for_snapshot(&g.key, snap, fp.as_deref(), ctx.scan_kind)
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
            if let FactRun::Failed(_) = self.run_one_collector(g, ctx, files.clone()).await {
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

    /// Run the fact collectors an `effort.finished` triggers, over the
    /// effort's end snapshot (the worktree as it stood at close), stamped
    /// with the effort.
    pub async fn run_effort_collectors(
        &self,
        thread_id: &ThreadId,
        effort_id: &EffortId,
        event: Option<Arc<oxplow_domain::StoredEvent>>,
    ) {
        let gauges: Vec<FactCollector> = self
            .fact_collectors()
            .into_iter()
            .filter(|g| g.runs_on(EFFORT_FINISHED))
            .filter(|g| {
                event
                    .as_ref()
                    .is_none_or(|e| crate::collector_triggers::matches_where(&g.trigger, e))
            })
            .collect();
        let gauges = self.runnable(gauges, event.as_deref()).await;
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
                EFFORT_FINISHED,
                snapshot_id.unwrap_or(0),
            )
            .await;
        ctx.event = event;
        // This trigger KNOWS the producing effort — stamp it so the capture is
        // attributable via `captures_for_effort` (tsk43; the pre-arc effort
        // subject default was removed without a replacement).
        ctx.effort_id = Some(effort_id.value());
        for g in &gauges {
            self.run_one_collector(g, &ctx, files.clone()).await;
        }
    }

    /// Run the fact collectors an event of any other type triggers, over
    /// the latest snapshot of the event's stream (the primary when it has
    /// none).
    pub async fn run_event_collectors(&self, event: Arc<oxplow_domain::StoredEvent>) {
        let event_type = event.envelope.event_type.clone();
        let collectors: Vec<FactCollector> = self
            .fact_collectors()
            .into_iter()
            .filter(|c| {
                c.runs_on(&event_type)
                    && crate::collector_triggers::matches_where(&c.trigger, &event)
            })
            .collect();
        let collectors = self.runnable(collectors, Some(&event)).await;
        if collectors.is_empty() {
            return;
        }
        let stream_val = event.envelope.anchors.stream_id.map_or(1, |s| s.value());
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
        let mut ctx = self
            .snapshot_context(stream_val, None, "on", snapshot_id.unwrap_or(0))
            .await;
        ctx.thread_id = event.envelope.anchors.thread_id.map(|t| t.value());
        ctx.effort_id = event.envelope.anchors.effort_id.map(|e| e.value());
        ctx.event = Some(event);
        for c in &collectors {
            self.run_one_collector(c, &ctx, files.clone()).await;
        }
    }

    /// Run one fact collector now, by owner and id, over the stream's latest
    /// snapshot (the `collector.sync` command). Returns the facts recorded,
    /// or why it couldn't run (unknown, an unapproved program, a missing
    /// script) or failed — never a silent zero.
    pub async fn run_collector_by_key(
        &self,
        owner: &str,
        key: &str,
        stream: Option<StreamId>,
        source: &str,
    ) -> Result<usize, String> {
        let metric = self
            .fact_collectors()
            .into_iter()
            .find(|m| m.owner == owner && m.key == key)
            .ok_or_else(|| format!("no fact collector `{owner}/{key}`"))?;
        if let Some(log) = self.run_log.as_ref() {
            let health_key = crate::collector_runner::plugin_key(owner, key);
            if let Some(reason) = log
                .health()
                .disabled_reason(&health_key)
                .await
                .map_err(|e| e.to_string())?
            {
                return Err(format!(
                    "collector `{owner}/{key}` is disabled: {reason}. A person can enable it \
                     again (`plugin.enable`, Settings → Extensions)."
                ));
            }
        }
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
        // Asked for explicitly: say why it can't run (an unapproved program,
        // a missing script) instead of quietly recording nothing.
        let mut ctx = self
            .snapshot_context(stream_val, None, "manual", snapshot_id.unwrap_or(0))
            .await;
        ctx.source = source.to_string();
        let runner = match self.fact_runner(&metric) {
            Ok(r) => r,
            Err(e) => {
                self.log_run(&metric, &ctx, &FactRun::Failed(e.clone()), 0, None)
                    .await;
                return Err(e);
            }
        };
        match self.run_fact_runner(&metric, runner, &ctx, files).await {
            FactRun::Recorded(n) => Ok(n),
            FactRun::Failed(e) => Err(e),
            FactRun::Skipped => Ok(0),
        }
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
        let content = self.content.clone();
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
                match content.read(f.storage, hash) {
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
    ) -> CollectorRunContext {
        let version = if snapshot_id > 0 {
            crate::file_ref_version::resolve(
                &self.snapshot_store,
                &*self.vcs,
                &self.project_dir,
                snapshot_id,
            )
            .await
            .ok()
        } else {
            None
        };
        CollectorRunContext {
            stream_val,
            thread_id,
            trigger,
            snapshot_id: (snapshot_id > 0).then_some(snapshot_id),
            closest_vcs_rev: version.as_ref().and_then(|v| v.closest_vcs_rev.clone()),
            vcs_rev_exact: version.as_ref().map(|v| v.vcs_rev_exact).unwrap_or(false),
            branch: self.current_branch().await,
            effort_id: None,
            scan_kind: "delta",
            event: None,
            source: "system".into(),
        }
    }

    /// What runs `c`: its embedded script for a built-in, its script read
    /// from its extension or the project, or — for a project's `exec`
    /// collector a person approved on this machine — its program.
    fn fact_runner(&self, c: &FactCollector) -> Result<FactRunner, String> {
        let script = || {
            collector_script_text(c, &self.project_dir).ok_or_else(|| {
                format!(
                    "collector `{}`: entry `{}` can't be read",
                    c.key,
                    c.entry.as_deref().unwrap_or_default()
                )
            })
        };
        Ok(match c.runtime {
            CollectorRuntime::Starlark => FactRunner::Starlark(script()?),
            CollectorRuntime::Jaq => FactRunner::Jaq(script()?),
            CollectorRuntime::Exec => {
                // A project collector's program comes from the repo: it runs
                // only once a person approved it on this machine (tsk331).
                use crate::exec_consent::{may_run, needs_approval, ProgramKind};
                let entry = c.entry.as_deref().unwrap_or_default();
                if !may_run(
                    &self.approvals,
                    &self.project_dir,
                    ProgramKind::Collector,
                    &c.key,
                    entry,
                    &[],
                ) {
                    return Err(needs_approval(ProgramKind::Collector, &c.key, entry));
                }
                FactRunner::Exec(vec![self
                    .project_dir
                    .join(entry)
                    .to_string_lossy()
                    .into_owned()])
            }
            CollectorRuntime::Read => {
                return Err(format!(
                    "collector `{}` reads a provider; it records no facts",
                    c.key
                ))
            }
        })
    }

    /// Run one fact collector: build its runner and run it with the
    /// file-map host. Best-effort — a collector that can't run (no consent,
    /// no script) is logged and recorded as failed. How it went.
    async fn run_one_collector(
        &self,
        gauge: &FactCollector,
        ctx: &CollectorRunContext,
        files: Arc<HashMap<String, String>>,
    ) -> FactRun {
        match self.fact_runner(gauge) {
            Ok(runner) => self.run_fact_runner(gauge, runner, ctx, files).await,
            Err(e) => {
                tracing::warn!(key = %gauge.key, error = %e, "fact collector: not run");
                self.log_run(gauge, ctx, &FactRun::Failed(e.clone()), 0, None)
                    .await;
                FactRun::Failed(e)
            }
        }
    }

    /// [`Self::run_one_collector`] with its runner already built: its input
    /// (`{report?, rows?, event?}`), the run under the sandbox budget, its
    /// facts recorded as one capture (or a failed capture), and its
    /// `collector_run` + `collector.synced@1`.
    async fn run_fact_runner(
        &self,
        gauge: &FactCollector,
        runner: FactRunner,
        ctx: &CollectorRunContext,
        files: Arc<HashMap<String, String>>,
    ) -> FactRun {
        let source = collector_source(gauge);
        let started = std::time::Instant::now();
        let outcome = match self.fact_input(gauge, ctx).await {
            Err(e) => Err(e),
            Ok(input) => {
                // A tree collector scans the WHOLE tree (hundreds of files,
                // tree-sitter each): the 5 s default sized for report parsers
                // silently timed out the broad-query ones on every full-tree
                // run (tsk47). They run detached under `spawn_blocking`, so a
                // generous ceiling costs nothing and still catches a runaway.
                let host = TreeHost::from_shared(files);
                tokio::task::spawn_blocking(move || runner.run(&input, host))
                    .await
                    .unwrap_or_else(|e| Err(format!("task failed: {e}")))
            }
        };
        let (run, capture) = match outcome {
            Ok(gauge_facts) => {
                tracing::debug!(
                    key = %gauge.key,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "fact collector: complete",
                );
                match self
                    .collector_capture(gauge, ctx, &source, &gauge_facts)
                    .await
                {
                    Some((capture, rows)) => (FactRun::Recorded(rows.len()), Some((capture, rows))),
                    None => (FactRun::Recorded(0), None),
                }
            }
            Err(e) => {
                // NOT a silent warn: a collector that fails leaves its metric
                // reading stale or empty, which is how two built-in metrics
                // went unnoticed for weeks. Record the failure durably.
                tracing::error!(
                    key = %gauge.key,
                    error = %e,
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    "fact collector: FAILED — its metric will read stale or empty",
                );
                let capture = self.failure_capture(gauge, ctx, &source, &e);
                (FactRun::Failed(e), Some((capture, Vec::new())))
            }
        };
        let elapsed = i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX);
        self.log_run(gauge, ctx, &run, elapsed, capture).await;
        run
    }

    /// A fact collector's input: its `report` parsed in its format, its
    /// `input` rows (the trigger's anchors bound), and the trigger event.
    async fn fact_input(
        &self,
        c: &FactCollector,
        ctx: &CollectorRunContext,
    ) -> Result<serde_json::Value, String> {
        let mut input = serde_json::Map::new();
        if let Some(report) = &c.report {
            // A report that isn't there yet is empty text, not a failure: the
            // script decides (a tool that hasn't run).
            let text =
                std::fs::read_to_string(self.project_dir.join(&report.path)).unwrap_or_default();
            input.insert(
                "report".into(),
                oxplow_collect_plugin::parse_report(&report.format, &text)
                    .map_err(|e| format!("collector `{}` report: {e}", c.key))?,
            );
        }
        if let Some(sql) = &c.input {
            let log = self.run_log.as_ref().ok_or_else(|| {
                format!("collector `{}`: no database to read `input` from", c.key)
            })?;
            let params = crate::collector_runner::anchor_params(
                ctx.event.as_deref(),
                crate::collector_runner::Anchored {
                    stream_id: Some(ctx.stream_val),
                    snapshot_id: ctx.snapshot_id,
                    effort_id: ctx.effort_id,
                    thread_id: ctx.thread_id,
                },
            );
            let rows = crate::collector_runner::input_rows(&log.layer, &c.key, sql, params).await?;
            input.insert("rows".into(), serde_json::Value::Array(rows));
        }
        if let Some(e) = &ctx.event {
            input.insert("event".into(), crate::collector_runner::event_input(e));
        }
        Ok(serde_json::Value::Object(input))
    }

    /// Record a fact collector's run: its `capture` (and facts), its
    /// `collector_run` row and `collector.synced@1` in one transaction
    /// (tsk712) — a run never lands without its record, and a redelivered
    /// event writes nothing. Without a run log (tests) the capture alone is
    /// written. A failure to record is logged.
    async fn log_run(
        &self,
        c: &FactCollector,
        ctx: &CollectorRunContext,
        run: &FactRun,
        elapsed_ms: i64,
        capture: Option<(oxplow_db::NewMetricCapture, Vec<oxplow_db::NewFact>)>,
    ) {
        let Some(log) = self.run_log.as_ref() else {
            if let (Some(facts), Some((capture, rows))) = (self.fact_store.as_ref(), capture) {
                if let Err(e) = facts.record_facts(capture, rows).await {
                    tracing::warn!(key = %c.key, error = %e, "fact collector: capture write failed");
                }
            }
            return;
        };
        let (status, facts, error) = match run {
            FactRun::Recorded(n) => ("ok", i64::try_from(*n).unwrap_or(i64::MAX), None),
            FactRun::Failed(e) => ("error", 0, Some(e.clone())),
            FactRun::Skipped => return,
        };
        // The plugin failure policy (P7.C2): the third failure in a row
        // disables it.
        let health = log.health();
        let key = crate::collector_runner::plugin_key(&c.owner, &c.key);
        let counted = match &error {
            None => {
                health
                    .succeeded(
                        &key,
                        Some(std::time::Duration::from_millis(elapsed_ms.max(0) as u64)),
                    )
                    .await
            }
            Some(e) => health.failed(&key, e).await.map(|_| ()),
        };
        if let Err(e) = counted {
            tracing::warn!(key = %c.key, error = %e, "fact collector: recording its health failed");
        }
        let trigger = match ctx.trigger {
            "manual" => "manual",
            _ => "on",
        };
        match log
            .record_with(
                crate::collector_runner::RunRecord {
                    owner: &c.owner,
                    id: &c.key,
                    trigger,
                    source: &ctx.source,
                    cause: ctx.event.as_deref().map(|e| (e.envelope.id.clone(), e.seq)),
                    status,
                    entities: Default::default(),
                    facts,
                    elapsed_ms,
                    error,
                },
                capture,
            )
            .await
        {
            Ok(_) => {
                if let Some(facts) = self.fact_store.as_ref() {
                    facts.facts_committed();
                }
            }
            Err(e) => {
                tracing::warn!(key = %c.key, error = %e, "fact collector: run record failed; its capture with it")
            }
        }
    }

    /// A FAILED gauge run as a `status = 'failed'` capture (tsk47), recorded
    /// with the run ([`Self::log_run`]).
    ///
    /// Two reasons this must be durable rather than a log line:
    /// 1. **Visibility.** A gauge that fails leaves its metric reading stale or empty
    ///    *forever*, and nothing said so — `oxplow.ts.console_calls` read empty for
    ///    weeks against a repo with 137 console calls. A metric that is obviously
    ///    broken is far better than one that is quietly wrong.
    /// 2. **It stops the boot loop.** The capture carries the gauge's fingerprint, so
    ///    `collector_is_stale` sees the current logic *was* attempted and doesn't demand a
    ///    fresh full-tree baseline on every single boot.
    ///
    /// It carries NO facts, and the read folds skip non-`done` captures — critical,
    /// because an empty capture over a *full-tree* snapshot restates every path, and
    /// would otherwise supersede everything and zero the metric.
    fn failure_capture(
        &self,
        gauge: &FactCollector,
        ctx: &CollectorRunContext,
        source: &str,
        error: &str,
    ) -> oxplow_db::NewMetricCapture {
        let mut capture = oxplow_db::NewMetricCapture::done(
            ctx.stream_val,
            gauge.key.clone(),
            source.to_string(),
        );
        capture.status = "failed".into();
        capture.error = Some(error.to_string());
        capture.thread_id = ctx.thread_id;
        capture.effort_id = ctx.effort_id;
        capture.scope = Some(gauge.scope());
        capture.trigger = Some(ctx.trigger.into());
        capture.snapshot_id = ctx.snapshot_id;
        capture.closest_vcs_rev = ctx.closest_vcs_rev.clone();
        capture.vcs_rev_exact = ctx.vcs_rev_exact;
        capture.branch = ctx.branch.clone();
        capture.producer_version = collector_fingerprint(gauge, &self.project_dir);
        capture.scan_kind = ctx.scan_kind.into();
        capture
    }

    /// A gauge's per-item `facts` (epic tsk12) as `fact` rows under one
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
    /// the producer's captures (tsk44). Builds the capture and its facts;
    /// [`Self::log_run`] writes them with the run's record.
    async fn collector_capture(
        &self,
        gauge: &FactCollector,
        ctx: &CollectorRunContext,
        source: &str,
        gauge_facts: &[CollectedFact],
    ) -> Option<(oxplow_db::NewMetricCapture, Vec<oxplow_db::NewFact>)> {
        let facts = self.fact_store.as_ref()?;
        // Resolve the measure catalog once (one query), then map each fact's key.
        let by_key: HashMap<String, i64> = match facts.list_measures().await {
            Ok(ms) => ms.into_iter().map(|m| (m.key, m.id)).collect(),
            Err(e) => {
                tracing::warn!(key = %gauge.key, error = %e, "gauge facts: measure catalog read failed");
                return None;
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
            if !gauge.facts.is_empty() && !gauge.facts.iter().any(|m| m == &gf.measure) {
                tracing::warn!(
                    key = %gauge.key, measure = %gf.measure,
                    "collector facts: measure not in the collector's `facts` — fact dropped"
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
        let capture = oxplow_db::NewMetricCapture {
            thread_id: ctx.thread_id,
            effort_id: ctx.effort_id,
            scope: Some(gauge.scope()),
            trigger: Some(ctx.trigger.into()),
            basis_ref: ctx.closest_vcs_rev.clone(),
            snapshot_id: ctx.snapshot_id,
            closest_vcs_rev: ctx.closest_vcs_rev.clone(),
            vcs_rev_exact: ctx.vcs_rev_exact,
            branch: ctx.branch.clone(),
            // Record WHICH LOGIC produced these facts, so a later script change is
            // detectable and can re-baseline instead of silently no-opping (tsk45).
            producer_version: collector_fingerprint(gauge, &self.project_dir),
            scan_kind: ctx.scan_kind.into(),
            ..oxplow_db::NewMetricCapture::done(
                ctx.stream_val,
                gauge.key.clone(),
                source.to_string(),
            )
        };
        Some((capture, rows))
    }
}

/// Map a resolved `ResolvedSpec` to a `metric_spec` write (for config-declared
/// metrics). A formula metric has no `source_measure`.
fn spec_to_new_spec(s: &ResolvedSpec) -> NewMetricSpec {
    let spec = NewMetricSpec {
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
        entity_json: None,
    };
    match &s.entity {
        Some(entity) => as_entity_spec(spec, entity),
        None => spec,
    }
}

/// Make `spec` an entity metric over `entity` (tsk322). An event metric (with
/// `time`) has no source measure: it's computed live. A state metric reads a
/// synthesized measure of its own key, which the `entity-metric` producer
/// feeds. The entity's own aggregation lives in `entity_json`; the stored
/// `aggregation` says how the series' points combine, which is what the fact
/// path and a range total read: `last` for a state metric (one fact per
/// capture, a level), `sum` for an event metric whose buckets add up
/// (count / sum; not count_distinct, tsk367), else `avg`.
fn as_entity_spec(mut spec: NewMetricSpec, entity: &oxplow_config::EntitySpec) -> NewMetricSpec {
    spec.source_measure = entity.time.is_none().then(|| spec.key.clone());
    spec.aggregation = if entity.time.is_none() {
        "last"
    } else if crate::entity_metrics::sums_across_buckets(&entity.aggregation) {
        "sum"
    } else {
        "avg"
    }
    .into();
    spec.filter_json = None;
    spec.formula = None;
    spec.entity_json = serde_json::to_string(entity).ok();
    if entity.time.is_some() && spec.display_kind == "gauge" {
        spec.display_kind = "event".into();
    }
    spec
}

/// Seed one dimension; an entity dimension whose SQL doesn't compile is
/// skipped with a warning instead of reaching the catalog.
async fn seed_dimension(
    facts: &SqliteFactStore,
    layer: &crate::sql_gateway::SqlGateway,
    nd: NewDimension,
) -> bool {
    let entity = nd
        .entity_json
        .as_deref()
        .and_then(|j| serde_json::from_str::<oxplow_config::EntityDimensionSpec>(j).ok());
    if let Some(dim) = &entity {
        let probe = oxplow_config::EntitySpec {
            view: dim.view.clone(),
            where_: None,
            time: None,
            value: None,
            aggregation: "count".into(),
        };
        if let Err(e) = crate::entity_metrics::check(layer, &probe, Some(dim)).await {
            tracing::warn!(key = %nd.key, error = %e, "entity dimension doesn't compile; skipping");
            return false;
        }
    }
    match facts.upsert_dimension(nd.clone()).await {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(key = %nd.key, error = %e, "failed to seed dimension");
            false
        }
    }
}

/// Ready an entity spec for the catalog: its SQL must compile, its sliceable
/// dims are the entity dimensions over its view, and a state metric gets its
/// synthesized measure. `false` (with a warning) keeps it out. A non-entity
/// spec passes through untouched.
async fn prepare_entity_spec(
    facts: &SqliteFactStore,
    layer: &crate::sql_gateway::SqlGateway,
    spec: &mut NewMetricSpec,
    entity_dims: &[(String, String)],
) -> bool {
    let Some(entity) = spec
        .entity_json
        .as_deref()
        .and_then(|j| serde_json::from_str::<oxplow_config::EntitySpec>(j).ok())
    else {
        return true;
    };
    if let Err(e) = crate::entity_metrics::check(layer, &entity, None).await {
        tracing::warn!(key = %spec.key, error = %e, "entity metric doesn't compile; skipping");
        return false;
    }
    let mut dims: Vec<String> = spec
        .sliceable_dims_json
        .as_deref()
        .and_then(|j| serde_json::from_str(j).ok())
        .unwrap_or_default();
    for (view, key) in entity_dims {
        if *view == entity.view && !dims.contains(key) {
            dims.push(key.clone());
        }
    }
    spec.sliceable_dims_json =
        (!dims.is_empty()).then(|| serde_json::to_string(&dims).unwrap_or_default());
    if entity.time.is_none() {
        let measure = NewMeasure {
            key: spec.key.clone(),
            title: spec.title.clone(),
            unit: spec.unit.clone(),
            subject_kind: None,
            temporal_semantics: "semi-additive".into(),
            capture_scope: "complete".into(),
            scope: spec.scope.clone(),
            description: spec.description.clone(),
        };
        if let Err(e) = facts.upsert_measure(measure).await {
            tracing::warn!(key = %spec.key, error = %e, "failed to seed entity metric measure");
            return false;
        }
    }
    true
}

/// The primary stream's id, which project-wide captures are recorded against.
async fn primary_stream(layer: &crate::sql_gateway::SqlGateway) -> Option<i64> {
    let out = layer
        .query_sql(
            "SELECT id FROM v_stream WHERE kind = 'primary'",
            vec![],
            Some(1),
        )
        .await
        .ok()?;
    match out.rows.first()?.first()? {
        oxplow_db::SqlCell::Int(id) => Some(*id),
        _ => None,
    }
}

/// Built-in entity dimensions (tsk322).
fn builtin_entity_dimensions() -> Vec<NewDimension> {
    let mut priority = NewDimension::categorical("work.priority", "Priority");
    priority.entity_json = Some(r#"{"view":"v_task","expr":"e.priority"}"#.into());
    vec![priority]
}

/// Built-in entity metrics over core views (tsk322).
fn builtin_entity_specs() -> Vec<NewMetricSpec> {
    let make = |key: &str, title: &str, description: &str, entity: oxplow_config::EntitySpec| {
        let mut s = NewMetricSpec::base(key, title, key, "sum");
        s.unit = Some("tasks".into());
        s.description = Some(description.into());
        s.category = Some("operational".into());
        as_entity_spec(s, &entity)
    };
    vec![
        make(
            "work.tasks_completed",
            "Tasks completed",
            "Tasks marked done, by the day they were completed.",
            oxplow_config::EntitySpec {
                view: "v_task".into(),
                where_: Some("status = 'done'".into()),
                time: Some("completed_at".into()),
                value: None,
                aggregation: "count".into(),
            },
        ),
        make(
            "work.open_tasks",
            "Open tasks",
            "Tasks ready, in progress or blocked, captured over time.",
            oxplow_config::EntitySpec {
                view: "v_task".into(),
                where_: Some("status IN ('ready', 'in_progress', 'blocked')".into()),
                time: None,
                value: None,
                aggregation: "count".into(),
            },
        ),
    ]
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
        // Doc coverage (tsk125) is a RATIO over oxplow.doc_coverage (num/den =
        // documented/public per file), not a count — so it's built inline
        // rather than via the count-only `spec()` helper.
        {
            let mut s = NewMetricSpec::base(
                "oxplow.doc_coverage",
                "Doc coverage",
                "oxplow.doc_coverage",
                "ratio",
            );
            s.unit = Some("%".into());
            s.direction = "higher-better".into();
            s.display_kind = "coverage".into();
            s.category = Some("coverage".into());
            s.description = Some(
                "% of public functions/methods with a doc comment — Σdocumented/Σpublic over oxplow.doc_coverage facts.".into(),
            );
            s
        },
        // Duplicated lines (tsk388): the sum over the whole-tree scan's
        // facts, one per side of each duplicate block.
        {
            let mut s = NewMetricSpec::base(
                "oxplow.duplicate_lines",
                "Duplicated lines",
                "oxplow.duplicate_lines",
                "sum",
            );
            s.unit = Some("lines".into());
            s.direction = "lower-better".into();
            s.display_kind = "findings".into();
            s.category = Some("static-quality".into());
            s.description = Some(
                "Lines in blocks duplicated elsewhere in the tree — sum over oxplow.duplicate_lines facts.".into(),
            );
            s
        },
    ]
}

/// Built-in metric SPECS for the per-language idiom gauges (epic tsk12, tsk30) —
/// `oxplow.rust.unsafe_blocks` and friends. Each is a `Sum(oxplow.ast_hit)`
/// filtered to its idiom via `dim_eq(oxplow.rule, <slug>)`; the per-file
/// `oxplow.ast_hit` facts each gauge emits (rule-tagged) sum back to the baked
/// `tree:.` headline — pinned by the equivalence test. The `<slug>` MUST match
/// the `rule` the gauge script emits.
fn builtin_ast_specs() -> Vec<NewMetricSpec> {
    let gauges = builtin_metrics();
    let ast_spec = |key: &str, title: &str, rule: &str, direction: &str| {
        let mut s = NewMetricSpec::base(key, title, "oxplow.ast_hit", "sum");
        s.unit = Some("count".into());
        s.filter_json = Some(format!("{{\"dim_eq\":[\"oxplow.rule\",\"{rule}\"]}}"));
        s.direction = direction.into();
        s.display_kind = "findings".into();
        s.category = Some("static-quality".into());
        // Read the language off the GAUGE rather than restating it here: both
        // metric surfaces section by language (Metric Settings off the gauge's,
        // Recorded Metrics off the spec's), so a duplicated slug is a silent
        // drift into two different groupings of the same metric. `language: ""`
        // (the language-agnostic code gauges) stays `None` — "" is not a
        // language, and `groupByLanguage` reads null/"" as its "General" bucket.
        s.language = gauges
            .iter()
            .find(|m| m.key == key)
            .map(|m| m.language)
            .filter(|l| !l.is_empty())
            .map(Into::into);
        s
    };
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

/// A filesystem-safe slug from a namespaced key (non-alphanumerics → `_`), for
/// naming the global scaffold's `<slug>.yaml` / `<slug>.star` files.
/// What [`MetricsService::metric_scaffold`] hands the agent to write.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricScaffold {
    pub key: String,
    /// Where the script goes, project-relative.
    pub script_path: String,
    /// The starter collector script.
    pub script: String,
    /// `measures:` / `collectors:` / `metrics:` entries to merge into
    /// `.oxplow/project.yaml` (appending to lists already there).
    pub project_yaml: String,
}

fn slugify(key: &str) -> String {
    key.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// The starter Starlark gauge from [`MetricsService::metric_scaffold`]. A
/// working tree-derived gauge that emits one per-file `<measure>` FACT per
/// matched file (TODO/FIXME count) — the metric spec (`sum` over `<measure>`)
/// charts it. Emits facts only (no baked sample), the clean substrate model
/// (epic tsk12); an `ast_query` example is in a comment for the author.
fn starter_collector_script(
    key: &str,
    measure: &str,
    glob: &str,
    language: Option<&str>,
) -> String {
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

/// What runs a fact collector.
enum FactRunner {
    Starlark(String),
    Jaq(String),
    /// A project's approved program: `argv`, the input as JSON on stdin.
    Exec(Vec<String>),
}

impl FactRunner {
    /// Run over `input` under the fact-collector budget and read its
    /// `{"facts": [...]}`. Blocking: call it from `spawn_blocking`.
    fn run(self, input: &serde_json::Value, host: TreeHost) -> Result<Vec<CollectedFact>, String> {
        use oxplow_collect_plugin::runtime::{run_exec, run_jaq, run_sandboxed};
        use oxplow_collect_plugin::{facts_of, run_fact_starlark};
        let budget = SandboxBudget::with_timeout(FACT_COLLECTOR_TIMEOUT);
        match self {
            FactRunner::Starlark(script) => run_fact_starlark(&script, input, host, &budget),
            FactRunner::Jaq(script) => {
                let input = input.clone();
                run_sandboxed(&budget, move || run_jaq(&script, &input)).and_then(facts_of)
            }
            FactRunner::Exec(argv) => {
                run_exec(&budget, &argv, &input.to_string()).and_then(facts_of)
            }
        }
        .map_err(|e| e.to_string())
    }
}

/// The script a fact collector runs — the embedded text for a built-in, its
/// entry read through its extension, or the project's file. `None` when it
/// can't be read.
fn collector_script_text(gauge: &FactCollector, root: &Path) -> Option<String> {
    if gauge.is_builtin() {
        return builtin_metrics()
            .iter()
            .find(|m| m.key == gauge.key)
            .map(|m| m.script.to_string());
    }
    let entry = gauge.entry.as_deref()?;
    if gauge.owner == oxplow_config::collectors::PROJECT {
        return std::fs::read_to_string(root.join(entry)).ok();
    }
    crate::extensions::read_extension_file(root, &gauge.owner, entry)
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
fn collector_fingerprint(gauge: &FactCollector, root: &Path) -> Option<String> {
    let script = collector_script_text(gauge, root)?;
    // Everything that can change what the collector produces. `facts` matters
    // because a measure dropped from the allow-list silently stops being
    // recorded. The `v1` material is what gauges hashed (runtime — empty for
    // a built-in — report format, args, report path, emits, script), so a
    // gauge migrated to a collector keeps its baseline. `input` is hashed
    // after it when set (a collector's own query).
    let runtime = match (gauge.is_builtin(), gauge.runtime) {
        (true, _) => "",
        (false, CollectorRuntime::Starlark) => "starlark",
        (false, CollectorRuntime::Jaq) => "jaq",
        (false, CollectorRuntime::Exec) => "exec",
        (false, CollectorRuntime::Read) => "read",
    };
    let report = gauge.report.as_ref();
    let mut material = format!(
        "v1\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}",
        runtime,
        report.map_or("text", |r| r.format.as_str()),
        "",
        report.map_or("", |r| r.path.as_str()),
        gauge.facts.join(","),
        script,
    );
    if let Some(sql) = &gauge.input {
        material.push_str(&format!("\u{0}input:{sql}"));
    }
    Some(crate::blob_store::BlobStore::hash(material.as_bytes()))
}

/// Trust label: in-process tiers are `observed` under a `metric:<key>` source;
/// the `exec` escape hatch is flagged `plugin-exec:<name>` (lower-trust).
fn collector_source(gauge: &FactCollector) -> String {
    if gauge.runtime == CollectorRuntime::Exec {
        format!("plugin-exec:{}", gauge.key)
    } else {
        format!("metric:{}", gauge.key)
    }
}

/// What [`MetricsService::rebuild_baseline`] did — observable so a command caller or a
/// test can assert on it instead of reading tracing logs (tsk50).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, specta::Type)]
pub struct BaselineReport {
    /// False when no gauge needed a baseline (a warm, up-to-date repo).
    pub ran: bool,
    pub snapshot_id: Option<i64>,
    pub collectors_run: usize,
    /// Gauges that failed during the sweep (empty on success). Non-empty means those
    /// metrics will read stale/empty — a visible failure, not a silent one.
    pub failed: Vec<String>,
}

/// `metrics.entity_states` (P7.B6): re-capture state entity metrics when
/// their rows may have moved — a work item written, a snapshot taken, a
/// collector run — throttled per metric (`capture_entity_states`).
pub const ENTITY_STATES: &str = "metrics.entity_states";

pub struct EntityStates {
    pub metrics: MetricsService,
}

#[async_trait::async_trait]
impl crate::event_pump::AsyncEventConsumer for EntityStates {
    fn name(&self) -> &'static str {
        ENTITY_STATES
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type.starts_with("work_item.")
            || event_type == "snapshot.taken"
            || event_type == "collector.synced"
    }

    async fn handle(
        &self,
        _event: &oxplow_domain::StoredEvent,
    ) -> Result<(), oxplow_domain::DomainError> {
        self.metrics.capture_entity_states(false).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {

    /// Switch metrics through the real path — the `metric.enable` command —
    /// then reseed, as the service's event loop does on `ConfigChanged`.
    async fn enable(svc: &crate::Services, keys: &[String], enabled: bool) {
        // The catalog the command checks keys against (boot seeds it).
        svc.metrics.seed_catalog().await;
        svc.commands
            .run(
                &oxplow_domain::Actor::Human,
                crate::commands::metric::ENABLE,
                serde_json::json!({ "keys": keys, "enabled": enabled }),
                false,
            )
            .await
            .unwrap();
        svc.metrics.seed_catalog().await;
    }

    /// P4.1 (tsk486): every built-in entity metric and dimension reads only
    /// published views — nothing its SQL names is a physical table.
    #[tokio::test]
    async fn builtin_entity_metrics_read_only_views() {
        let layer = crate::sql_gateway::SqlGateway::new(oxplow_db::Database::in_memory());
        for spec in builtin_entity_specs() {
            let entity: oxplow_config::EntitySpec =
                serde_json::from_str(spec.entity_json.as_deref().unwrap()).unwrap();
            let reads = crate::entity_metrics::check(&layer, &entity, None)
                .await
                .unwrap();
            assert!(reads.tables.is_empty(), "{}: {:?}", spec.key, reads.tables);
            assert!(!reads.models.is_empty(), "{}", spec.key);
        }
        for dim in builtin_entity_dimensions() {
            let d: oxplow_config::EntityDimensionSpec =
                serde_json::from_str(dim.entity_json.as_deref().unwrap()).unwrap();
            let probe = oxplow_config::EntitySpec {
                view: d.view.clone(),
                where_: None,
                time: None,
                value: None,
                aggregation: "count".into(),
            };
            let reads = crate::entity_metrics::check(&layer, &probe, Some(&d))
                .await
                .unwrap();
            assert!(reads.tables.is_empty(), "{}: {:?}", dim.key, reads.tables);
        }
    }
    use super::*;
    use oxplow_domain::refs::build::work_item_ref;

    /// A distinct count's buckets don't add up, so its points aren't
    /// stored as a `sum` (the detail page would total them) (tsk367).
    #[test]
    fn a_distinct_event_metric_is_not_summed_across_buckets() {
        let entity = |aggregation: &str| oxplow_config::EntitySpec {
            view: "v_task".into(),
            where_: None,
            time: Some("completed_at".into()),
            value: Some("e.priority".into()),
            aggregation: aggregation.into(),
        };
        let stored = |aggregation: &str| {
            as_entity_spec(
                NewMetricSpec::base("k", "K", "k", "sum"),
                &entity(aggregation),
            )
            .aggregation
        };
        assert_eq!(stored("count"), "sum");
        assert_eq!(stored("sum"), "sum");
        assert_eq!(stored("count_distinct"), "avg");
        assert_eq!(stored("max"), "avg");
    }

    /// A `MetricsService` over a real in-memory `Services` + git repo.
    async fn fixture() -> (Arc<crate::Services>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
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
    /// that hands `run_one_collector` a map must create the matching snapshot — otherwise
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
                content_hash: None,
            })
            .collect();
        if !rows.is_empty() {
            svc.snapshot_store.capture_batch(rows).await.unwrap();
        }
        snap
    }

    /// P7.B3: a snapshot runs each enabled fact collector exactly once.
    /// The `collector.triggers` consumer is the one path (the event loop no
    /// longer runs them), and a redelivered `snapshot.taken` runs nothing
    /// again — no second capture, no second `collector.synced`.
    #[tokio::test]
    async fn a_snapshot_runs_each_enabled_collector_exactly_once() {
        let (svc, dir) = fixture().await;
        std::fs::write(
            dir.path().join("once.star"),
            "def transform(input):\n    return {\"facts\": [{\"measure\": \"oxplow.ast_hit\", \"value\": 1, \"rule\": \"once\", \"subject\": \"tree:.\"}]}\n",
        )
        .unwrap();
        let (specs, errors) = oxplow_config::collectors::parse_collectors(
            oxplow_config::collectors::PROJECT,
            &serde_yaml::from_str(
                "- { id: repo.once, runtime: starlark, entry: once.star, trigger: { on: [snapshot.taken] }, facts: [oxplow.ast_hit] }",
            )
            .unwrap(),
            &|_| true,
        );
        assert!(errors.is_empty(), "{errors:?}");
        svc.config.write().unwrap().collectors = specs;
        let snap =
            snapshot_with_files(&svc, &[("src/a.rs", oxplow_db::SnapshotStorage::Oxplow)]).await;
        let event = log_take(&svc, snap, SnapshotTrigger::TurnEnd, false, 1).await;
        let consumer = crate::collector_triggers::CollectorTriggers::new(Arc::downgrade(&svc));
        use crate::event_pump::AsyncEventConsumer as _;
        assert!(consumer.handles("snapshot.taken"));
        consumer.handle(&event).await.unwrap();
        consumer.handle(&event).await.unwrap();
        let counts: (i64, i64) = svc
            .db
            .read(|c| {
                c.query_row(
                    "SELECT (SELECT count(*) FROM metric_capture WHERE producer = 'repo.once'),
                            (SELECT count(*) FROM event_log WHERE type = 'collector.synced')",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(counts, (1, 1), "one capture and one run for one snapshot");
    }

    use oxplow_domain::snapshot::SnapshotTrigger;

    /// Log `snapshot.taken` for snapshot `snap` on stream 1, as the take
    /// that recorded it (or didn't) would.
    async fn log_take(
        svc: &crate::Services,
        snap: i64,
        trigger: SnapshotTrigger,
        unchanged: bool,
        file_count: u32,
    ) -> oxplow_domain::StoredEvent {
        let env = oxplow_domain::Envelope::typed::<oxplow_domain::events::schema::SnapshotTaken>(
            "system",
            &oxplow_domain::events::schema::SnapshotTakenV1 {
                stream: "stream:1".into(),
                snapshot: format!("snapshot:{snap}"),
                parent: None,
                trigger,
                unchanged,
                file_count,
                elapsed_ms: 1,
                budget_ms: None,
                over_budget: false,
            },
        )
        .with_anchors(oxplow_domain::events::Anchors {
            stream_id: Some(StreamId::new(1)),
            snapshot_id: Some(snap),
            ..Default::default()
        });
        let id = env.id.clone();
        svc.event_log_store.append(env).await.unwrap();
        svc.event_log_store.get(id).await.unwrap().unwrap()
    }

    /// A snapshot on stream 1 recording `files` with their content (`None`
    /// deletes the path).
    async fn snapshot_with_content(svc: &crate::Services, files: &[(&str, Option<&str>)]) -> i64 {
        let snap = svc
            .snapshot_store
            .create_snapshot(StreamId::new(1))
            .await
            .unwrap();
        let rows = files
            .iter()
            .map(|(path, text)| oxplow_db::FileSnapshot {
                id: 0,
                stream_id: StreamId::new(1),
                path: (*path).to_string(),
                blob_hash: text.map(|t| svc.blobs.write(t.as_bytes()).unwrap()),
                size_bytes: text.map_or(0, |t| t.len() as i64),
                captured_at: oxplow_domain::Timestamp::now(),
                storage: match text {
                    Some(_) => oxplow_db::SnapshotStorage::Oxplow,
                    None => oxplow_db::SnapshotStorage::Deleted,
                },
                snapshot_id: Some(snap),
                mtime_ms: None,
                content_hash: None,
            })
            .collect();
        svc.snapshot_store.capture_batch(rows).await.unwrap();
        snap
    }

    /// P7.B5 (tsk388): `oxplow.duplicate_lines` is restated over the whole
    /// tree on every ref move — a take that recorded nothing included —
    /// and a clean tree clears it.
    #[tokio::test]
    async fn a_ref_move_restates_duplicate_lines_over_the_whole_tree() {
        const BODY: &str = "pub fn compute(input: &[i64]) -> i64 {\n\
            \x20   let mut total = 0;\n\
            \x20   for value in input {\n\
            \x20       if *value > 0 {\n\
            \x20           total += *value;\n\
            \x20       } else {\n\
            \x20           total -= *value;\n\
            \x20       }\n\
            \x20   }\n\
            \x20   total * 2 + 1\n\
            }\n";
        let (svc, _dir) = fixture().await;
        svc.config.write().unwrap().metrics.push(MetricEntry {
            use_key: Some("oxplow.duplicate_lines".into()),
            ..Default::default()
        });
        let consumer = crate::collector_triggers::CollectorTriggers::new(Arc::downgrade(&svc));
        use crate::event_pump::AsyncEventConsumer as _;
        let measure = svc
            .fact_store
            .get_measure("oxplow.duplicate_lines")
            .await
            .unwrap()
            .unwrap();
        let current = || async {
            let mut subjects: Vec<String> = svc
                .metric_engine
                .current_facts(&measure)
                .await
                .unwrap()
                .into_iter()
                .filter_map(|f| f.subject_ref)
                .collect();
            subjects.sort();
            subjects
        };
        let captures = || async {
            svc.db
                .read(|c| {
                    c.query_row(
                        "SELECT count(*) FROM metric_capture WHERE producer = 'oxplow.duplicate_lines'",
                        [],
                        |r| r.get::<_, i64>(0),
                    )
                    .map_err(oxplow_db::map_sql_err)
                })
                .await
                .unwrap()
        };

        // The copy lands in one take, then a second take records only one
        // side: neither is a ref move.
        let first = snapshot_with_content(
            &svc,
            &[
                ("src/a.rs", Some(BODY)),
                ("src/other.rs", Some("fn other() {}\n")),
            ],
        )
        .await;
        let second = snapshot_with_content(&svc, &[("src/b.rs", Some(BODY))]).await;
        for (snap, trigger) in [
            (first, SnapshotTrigger::Startup),
            (second, SnapshotTrigger::TurnEnd),
        ] {
            consumer
                .handle(&log_take(&svc, snap, trigger, false, 1).await)
                .await
                .unwrap();
        }
        assert_eq!(captures().await, 0, "only a ref move runs it");

        // A ref move on the unchanged tree reads all of it, not the
        // second take's one file.
        let moved = log_take(&svc, second, SnapshotTrigger::GitRefs, true, 0).await;
        consumer.handle(&moved).await.unwrap();
        let both = current().await;
        assert_eq!(both.len(), 2, "both sides of the copy: {both:?}");
        assert!(both[0].starts_with("src/a.rs:") && both[1].starts_with("src/b.rs:"));

        // The copy goes; the next ref move clears the metric.
        let third = snapshot_with_content(&svc, &[("src/b.rs", None)]).await;
        consumer
            .handle(&log_take(&svc, third, SnapshotTrigger::Quiet, false, 1).await)
            .await
            .unwrap();
        assert_eq!(current().await.len(), 2, "a save doesn't restate it");
        consumer
            .handle(&log_take(&svc, third, SnapshotTrigger::GitRefs, true, 0).await)
            .await
            .unwrap();
        assert!(current().await.is_empty(), "an empty capture clears it");
        assert_eq!(captures().await, 2);

        // P7 review (tsk709): a second clean restate is another empty
        // capture. It is history, not a baseline: the earlier captures and
        // the stream's cube stay.
        let measure_id = measure.id;
        svc.db
            .transaction(move |c| {
                c.execute(
                    "INSERT INTO metric_cube_state (measure_id, stream_id, branch, last_capture_id, last_captured_at)
                     VALUES (?1, 1, '', 1, '2026-01-01T00:00:00.000000Z')",
                    [measure_id],
                )
                .map(|_| ())
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        let fourth =
            snapshot_with_content(&svc, &[("src/other.rs", Some("fn other() {}\n"))]).await;
        consumer
            .handle(&log_take(&svc, fourth, SnapshotTrigger::GitRefs, true, 0).await)
            .await
            .unwrap();
        assert_eq!(captures().await, 3, "a clean restate prunes nothing");
        let cube_states: i64 = svc
            .db
            .read(|c| {
                c.query_row(
                    "SELECT count(*) FROM metric_cube_state WHERE stream_id = 1",
                    [],
                    |r| r.get(0),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(cube_states, 1, "the stream's cube survives a clean restate");
    }

    /// P7 review (tsk712): a fact collector's capture, its `collector_run`
    /// row and its `collector.synced@1` commit together — when the run
    /// record can't be written, no capture lands either, so a redelivered
    /// event can't record the run twice.
    #[tokio::test]
    async fn a_fact_collectors_capture_and_run_record_commit_together() {
        let (svc, dir) = fixture().await;
        std::fs::write(
            dir.path().join("once.star"),
            "def transform(input):\n    return {\"facts\": [{\"measure\": \"oxplow.ast_hit\", \"value\": 1, \"rule\": \"once\", \"subject\": \"tree:.\"}]}\n",
        )
        .unwrap();
        let (specs, errors) = oxplow_config::collectors::parse_collectors(
            oxplow_config::collectors::PROJECT,
            &serde_yaml::from_str(
                "- { id: repo.once, runtime: starlark, entry: once.star, trigger: { on: [snapshot.taken] }, facts: [oxplow.ast_hit] }",
            )
            .unwrap(),
            &|_| true,
        );
        assert!(errors.is_empty(), "{errors:?}");
        svc.config.write().unwrap().collectors = specs;
        let snap =
            snapshot_with_files(&svc, &[("src/a.rs", oxplow_db::SnapshotStorage::Oxplow)]).await;
        let event = log_take(&svc, snap, SnapshotTrigger::TurnEnd, false, 1).await;
        // The run record can't be written.
        svc.db
            .transaction(|c| {
                c.execute_batch(
                    "CREATE TRIGGER no_runs BEFORE INSERT ON collector_run
                     BEGIN SELECT RAISE(ABORT, 'no run records'); END;",
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        let consumer = crate::collector_triggers::CollectorTriggers::new(Arc::downgrade(&svc));
        use crate::event_pump::AsyncEventConsumer as _;
        let _ = consumer.handle(&event).await;
        let count = |sql: &'static str| {
            let svc = svc.clone();
            async move {
                svc.db
                    .read(move |c| {
                        c.query_row(sql, [], |r| r.get::<_, i64>(0))
                            .map_err(oxplow_db::map_sql_err)
                    })
                    .await
                    .unwrap()
            }
        };
        assert_eq!(
            count("SELECT count(*) FROM metric_capture WHERE producer = 'repo.once'").await,
            0,
            "no capture without its run record"
        );
        assert_eq!(
            count("SELECT count(*) FROM event_log WHERE type = 'collector.synced'").await,
            0
        );
        // Once it can be written, the redelivered event records the run once.
        svc.db
            .transaction(|c| {
                c.execute_batch("DROP TRIGGER no_runs")
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        consumer.handle(&event).await.unwrap();
        consumer.handle(&event).await.unwrap();
        assert_eq!(
            count("SELECT count(*) FROM metric_capture WHERE producer = 'repo.once'").await,
            1
        );
    }

    fn starlark_gauge(key: &str, entry_file: &str) -> FactCollector {
        starlark_gauge_emits(key, entry_file, Vec::new())
    }

    /// A project Starlark fact collector with an explicit `facts`
    /// allow-list, run on every snapshot.
    fn starlark_gauge_emits(key: &str, entry_file: &str, facts: Vec<String>) -> FactCollector {
        FactCollector {
            key: key.into(),
            owner: oxplow_config::collectors::PROJECT.into(),
            trigger: Trigger::On {
                events: vec![SNAPSHOT_TAKEN.into()],
                filter: Default::default(),
            },
            facts,
            runtime: CollectorRuntime::Starlark,
            entry: Some(entry_file.into()),
            report: None,
            input: None,
            after: Vec::new(),
            whole_tree: false,
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

        let ctx = CollectorRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "on-snapshot",
            snapshot_id: Some(42),
            closest_vcs_rev: Some("abc1234".into()),
            vcs_rev_exact: true,
            branch: Some("metrics-substrate".into()),
            effort_id: None,
            scan_kind: "delta",
            event: None,
            source: "system".into(),
        };
        // Only src/a.rs has unsafe blocks → one fact recorded.
        let count = svc
            .metrics
            .run_one_collector(&metric, &ctx, Arc::new(files))
            .await;
        assert_eq!(count, FactRun::Recorded(1));

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
        assert_eq!(facts[0].closest_vcs_rev.as_deref(), Some("abc1234"));
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
        let ctx = |snapshot_id: i64| CollectorRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "on-snapshot",
            snapshot_id: Some(snapshot_id),
            closest_vcs_rev: None,
            vcs_rev_exact: false,
            branch: None,
            effort_id: None,
            scan_kind: "delta",
            event: None,
            source: "system".into(),
        };

        // Scan 1: src/a.rs has one unsafe block.
        let s1 =
            snapshot_with_files(&svc, &[("src/a.rs", oxplow_db::SnapshotStorage::Oxplow)]).await;
        let dirty = HashMap::from([(
            "src/a.rs".to_string(),
            "fn a() { unsafe { x(); } }".to_string(),
        )]);
        svc.metrics
            .run_one_collector(&gauge, &ctx(s1), Arc::new(dirty))
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
            .run_one_collector(&gauge, &ctx(s2), Arc::new(clean))
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

    fn sweep_ctx() -> CollectorRunContext {
        CollectorRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "on-snapshot",
            snapshot_id: None,
            closest_vcs_rev: None,
            vcs_rev_exact: false,
            branch: None,
            effort_id: None,
            scan_kind: "delta",
            event: None,
            source: "system".into(),
        }
    }

    #[tokio::test]
    async fn rebuild_baseline_reads_the_whole_repo_end_to_end() {
        // THE test that would have caught all four metrics bugs (tsk47/48/49) without a
        // restart: real files on disk → full-tree snapshot → every gauge → fold →
        // repo-wide headline, driven through the same `rebuild_baseline` boot
        // uses. It runs TWO gauges sharing `oxplow.ast_hit` — the exact shape tsk49
        // hid in — so a single-gauge unit test could not have found it.
        let (svc, dir) = fixture().await;
        svc.metrics.seed_catalog().await;
        enable(
            &svc,
            &[
                "oxplow.rust.unsafe_blocks".into(),
                "oxplow.ts.console_calls".into(),
            ],
            true,
        )
        .await;

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

        let report = svc.metrics.rebuild_baseline(true).await.unwrap();
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
        let warm = svc.metrics.rebuild_baseline(false).await.unwrap();
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
        enable(&svc, &["oxplow.rust.unsafe_blocks".into()], true).await;
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "fn a() { unsafe { x(); } }\n").unwrap();

        let first = svc.metrics.rebuild_baseline(true).await.unwrap();
        assert!(first.ran);
        assert!(first.failed.is_empty(), "{:?}", first.failed);
        let latest_after_first = svc
            .snapshot_store
            .latest_snapshot_id_for_stream(oxplow_domain::StreamId::new(1))
            .await
            .unwrap();

        // Second forced rebuild: nothing on disk changed → no new snapshot,
        // and the kind-scoped idempotency guard skips the re-scan.
        let second = svc.metrics.rebuild_baseline(true).await.unwrap();
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
        // A built-in is only in `fact_collectors` when its metric is enabled — enable the
        // two built-ins this test drives.
        enable(
            &svc,
            &[
                "oxplow.rust.unsafe_blocks".into(),
                "oxplow.ts.console_calls".into(),
            ],
            true,
        )
        .await;
        let ctx = |snapshot_id: i64| CollectorRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "on-snapshot",
            snapshot_id: Some(snapshot_id),
            closest_vcs_rev: None,
            vcs_rev_exact: false,
            branch: None,
            effort_id: None,
            scan_kind: "delta",
            event: None,
            source: "system".into(),
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
            .run_one_collector(
                &builtin_gauge_fixture("oxplow.rust.unsafe_blocks"),
                &full_ctx,
                files,
            )
            .await;

        // console_calls has only ever run on a tiny delta.
        let small =
            snapshot_with_files(&svc, &[("f0.tsx", oxplow_db::SnapshotStorage::Oxplow)]).await;
        svc.metrics
            .run_one_collector(
                &builtin_gauge_fixture("oxplow.ts.console_calls"),
                &ctx(small),
                Arc::new(HashMap::from([("f0.tsx".to_string(), "ok".to_string())])),
            )
            .await;

        let needing = svc.metrics.collectors_needing_baseline(1).await;
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
            .run_collector_sweep(&[gauge], &sweep_ctx(), big_corpus(120))
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
            .run_collector_sweep(&[gauge], &sweep_ctx(), big_corpus(3))
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
            .run_collector_sweep(&[gauge], &sweep_ctx(), big_corpus(120))
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
        let ctx = CollectorRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "on-snapshot",
            snapshot_id: Some(snap),
            closest_vcs_rev: None,
            vcs_rev_exact: false,
            branch: None,
            effort_id: None,
            scan_kind: "delta",
            event: None,
            source: "system".into(),
        };
        let files = Arc::new(HashMap::from([("a.rs".to_string(), "x".to_string())]));
        svc.metrics
            .run_one_collector(&gauge, &ctx, files.clone())
            .await;

        // Same script → the recorded fingerprint still matches → nothing to redo.
        assert!(
            !svc.metrics.collector_is_stale(&gauge, 1).await,
            "an unchanged gauge must not force a re-baseline on every boot"
        );

        // Now the author fixes the gauge's logic (a different value).
        write(
            "def transform(input):\n    \
             return {\"facts\": [{\"measure\": \"oxplow.todo\", \"value\": 5.0, \
             \"subject\": \"file:a.rs\", \"path\": \"a.rs\"}]}\n",
        );
        assert!(
            svc.metrics.collector_is_stale(&gauge, 1).await,
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
        let ctx = |snapshot_id: i64| CollectorRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "on-snapshot",
            snapshot_id: Some(snapshot_id),
            closest_vcs_rev: None,
            vcs_rev_exact: false,
            branch: None,
            effort_id: None,
            scan_kind: "delta",
            event: None,
            source: "system".into(),
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
            .run_one_collector(&gauge, &ctx(base), Arc::new(full_tree))
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
            .run_one_collector(&gauge, &ctx(delta), Arc::new(changed))
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
        let ctx = |snapshot_id: i64| CollectorRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "on-snapshot",
            snapshot_id: Some(snapshot_id),
            closest_vcs_rev: None,
            vcs_rev_exact: false,
            branch: None,
            effort_id: None,
            scan_kind: "delta",
            event: None,
            source: "system".into(),
        };

        let s1 =
            snapshot_with_files(&svc, &[("src/a.rs", oxplow_db::SnapshotStorage::Oxplow)]).await;
        let dirty = HashMap::from([(
            "src/a.rs".to_string(),
            "fn a() { unsafe { x(); } }".to_string(),
        )]);
        svc.metrics
            .run_one_collector(&gauge, &ctx(s1), Arc::new(dirty))
            .await;

        // A later commit touched nothing this gauge scans: an empty snapshot + an
        // empty file map.
        let s2 = snapshot_with_files(&svc, &[]).await;
        svc.metrics
            .run_one_collector(&gauge, &ctx(s2), Arc::new(HashMap::new()))
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
        // `record_collector_facts` stamps the capture's `effort_id` so the T-D
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
        let effort = svc
            .effort_store
            .start(&work_item_ref(task), &thread, None)
            .await
            .unwrap();

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
        let ctx = CollectorRunContext {
            stream_val: 1,
            thread_id: Some(1),
            trigger: "on-effort-complete",
            snapshot_id: None,
            closest_vcs_rev: None,
            vcs_rev_exact: false,
            branch: None,
            effort_id: Some(effort.id.value()),
            scan_kind: "delta",
            event: None,
            source: "system".into(),
        };
        let count = svc
            .metrics
            .run_one_collector(&metric, &ctx, Arc::new(HashMap::new()))
            .await;
        assert_eq!(count, FactRun::Recorded(1));
        let caps = svc
            .fact_store
            .captures_for_effort(effort.id.value())
            .await
            .unwrap();
        assert_eq!(caps.len(), 1, "the capture is stamped with the effort");
        assert_eq!(caps[0].producer, "acme.effgauge");
    }

    #[tokio::test]
    async fn effort_finished_runs_on_effort_complete_gauges() {
        // Effort-triggered collectors run from the `collector.triggers` pump
        // consumer on `effort.finished` (P7.B3), not a direct call from
        // TaskService.
        use oxplow_domain::stores::TaskStore as _;
        use oxplow_domain::{Task, TaskActorKind, TaskAuthor, TaskId, TaskPriority, TaskStatus};
        let (svc, dir) = fixture().await;
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
        let effort = svc
            .effort_store
            .start(&work_item_ref(task), &thread, None)
            .await
            .unwrap();
        crate::collector_triggers::register(&svc);
        std::fs::create_dir_all(dir.path().join("oxplow/metrics")).unwrap();
        std::fs::write(
            dir.path().join("oxplow/metrics/eff.star"),
            "def transform(input):\n    return {\"facts\": [{\"measure\": \"oxplow.ast_hit\", \"value\": 1, \"rule\": \"eff\", \"subject\": \"tree:.\"}]}\n",
        )
        .unwrap();
        let (specs, errors) = oxplow_config::collectors::parse_collectors(
            oxplow_config::collectors::PROJECT,
            &serde_yaml::from_str(
                "- { id: acme.effgauge, runtime: starlark, entry: oxplow/metrics/eff.star, trigger: { on: [effort.finished] }, facts: [oxplow.ast_hit] }",
            )
            .unwrap(),
            &|_| true,
        );
        assert!(errors.is_empty(), "{errors:?}");
        svc.config.write().unwrap().collectors = specs;
        svc.tasks
            .update(
                task,
                crate::task_service::UpdateTaskChanges {
                    status: Some(TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        for _ in 0..100 {
            let caps = svc
                .fact_store
                .captures_for_effort(effort.id.value())
                .await
                .unwrap();
            if caps.iter().any(|c| c.producer == "acme.effgauge") {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("the on-effort-complete gauge didn't run after effort.finished");
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
        let ctx = CollectorRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "on-snapshot",
            snapshot_id: None,
            closest_vcs_rev: None,
            vcs_rev_exact: false,
            branch: None,
            effort_id: None,
            scan_kind: "delta",
            event: None,
            source: "system".into(),
        };
        let count = svc
            .metrics
            .run_one_collector(&metric, &ctx, Arc::new(files))
            .await;
        assert_eq!(count, FactRun::Recorded(1), "one function → one fact");

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

    /// A built-in fact collector — `run_one_collector` runs its embedded
    /// script (never a project-disk file).
    fn builtin_gauge_fixture(key: &str) -> FactCollector {
        FactCollector::builtin(
            &builtin_metrics()
                .into_iter()
                .find(|m| m.key == key)
                .expect("a built-in"),
        )
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
    async fn code_collector_facts_reaggregate_to_the_expected_headline() {
        // The keystone proof of the inversion (epic tsk12): a metric SPEC computed
        // over the per-item FACTS the gauge emitted == the expected gauge total,
        // for every bundled code metric. This is what let the reads flip to the
        // engine (T-C3) and the baked sample be removed (T-C3b).
        let (svc, _dir) = fixture().await;
        // Seed the built-in specs (count-over-facts) into the catalog.
        svc.metrics.seed_catalog().await;

        let files = Arc::new(equivalence_corpus());
        let ctx = CollectorRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "on-snapshot",
            snapshot_id: Some(7),
            closest_vcs_rev: Some("abc1234".into()),
            vcs_rev_exact: true,
            branch: Some("main".into()),
            effort_id: None,
            scan_kind: "delta",
            event: None,
            source: "system".into(),
        };
        for key in [
            "oxplow.fn_count",
            "oxplow.high_complexity_fns",
            "oxplow.long_functions",
            "oxplow.todos",
        ] {
            svc.metrics
                .run_one_collector(&builtin_gauge_fixture(key), &ctx, files.clone())
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
    return {"facts": [
        {"measure": "oxplow.complexity", "value": 5, "subject": "symbol:src/a.rs::foo"},
        {"measure": "acme.undefined", "value": 9, "subject": "symbol:src/a.rs::bar"},
    ]}
"#,
        )
        .unwrap();
        let metric = starlark_gauge("acme.mixed_facts", "oxplow/metrics/mixed.star");
        let ctx = CollectorRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "manual",
            snapshot_id: None,
            closest_vcs_rev: None,
            vcs_rev_exact: false,
            branch: None,
            effort_id: None,
            scan_kind: "delta",
            event: None,
            source: "system".into(),
        };
        svc.metrics
            .run_one_collector(&metric, &ctx, Arc::new(HashMap::new()))
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
        let ctx = CollectorRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "manual",
            snapshot_id: None,
            closest_vcs_rev: None,
            vcs_rev_exact: false,
            branch: None,
            effort_id: None,
            scan_kind: "delta",
            event: None,
            source: "system".into(),
        };
        svc.metrics
            .run_one_collector(&gauge, &ctx, Arc::new(HashMap::new()))
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
        let ctx = CollectorRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "manual",
            snapshot_id: None,
            closest_vcs_rev: None,
            vcs_rev_exact: false,
            branch: None,
            effort_id: None,
            scan_kind: "delta",
            event: None,
            source: "system".into(),
        };
        svc.metrics
            .run_one_collector(&gauge, &ctx, Arc::new(HashMap::new()))
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
    async fn per_language_collector_facts_reaggregate_through_the_spec() {
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
        let ctx = CollectorRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "on-snapshot",
            snapshot_id: Some(9),
            closest_vcs_rev: Some("def5678".into()),
            vcs_rev_exact: true,
            branch: Some("main".into()),
            effort_id: None,
            scan_kind: "delta",
            event: None,
            source: "system".into(),
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
                .run_one_collector(&builtin_gauge_fixture(key), &ctx, files.clone())
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
        let ctx = CollectorRunContext {
            stream_val: 1,
            thread_id: None,
            trigger: "on-snapshot",
            snapshot_id: Some(snap),
            closest_vcs_rev: None,
            vcs_rev_exact: false,
            branch: Some("main".into()),
            effort_id: None,
            scan_kind: "delta",
            event: None,
            source: "system".into(),
        };
        svc.metrics
            .run_one_collector(
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
        let groups = |dim: &'static str| {
            let engine = engine.clone();
            let spec = spec.clone();
            async move {
                engine
                    .series_for_spec(&spec, Some(dim))
                    .await
                    .unwrap()
                    .into_iter()
                    .map(|p| (p.group, p.value))
                    .collect::<Vec<_>>()
            }
        };
        let by_language = groups("oxplow.language").await;
        assert_eq!(
            by_language.len(),
            1,
            "one language group, got {by_language:?}"
        );
        assert_eq!(by_language[0].0.as_deref(), Some("rust"));
        // The bare key still slices identically.
        assert_eq!(groups("language").await, by_language);
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
    async fn builtin_ast_specs_carry_the_language_their_collector_declares() {
        // tsk81: the idiom specs are seeded with the SAME language slug their
        // built-in gauge declares, so the two metric surfaces can't disagree
        // about what language a metric is — Metric Settings sections by the
        // gauge's `language`, Recorded Metrics by the spec's. A spec left at the
        // `NewMetricSpec::base` default (`None`) silently collapses every idiom
        // metric into the "General" bucket and the per-language split no-ops.
        let (svc, _dir) = fixture().await;
        svc.metrics.seed_catalog().await;
        for (key, want) in [
            ("oxplow.rust.unsafe_blocks", "rust"),
            ("oxplow.rust.panic_macros", "rust"),
            // The key segment is `ts`, but the language slug is `typescript` —
            // the gauge is the authority, not the key.
            ("oxplow.ts.any_usage", "typescript"),
            ("oxplow.ts.console_calls", "typescript"),
            ("oxplow.clojure.defn_count", "clojure"),
            ("oxplow.csharp.empty_catch", "csharp"),
        ] {
            let spec = svc
                .fact_store
                .get_spec(key)
                .await
                .unwrap()
                .unwrap_or_else(|| panic!("{key} spec seeded"));
            assert_eq!(spec.language.as_deref(), Some(want), "{key} language");
            assert_eq!(
                spec.language.as_deref(),
                builtin_metrics()
                    .iter()
                    .find(|m| m.key == key)
                    .map(|m| m.language),
                "{key} spec language must match its gauge's",
            );
        }
        // The language-agnostic code gauges declare `language: ""` and must stay
        // language-less (they sweep every source file) — "" is not a language.
        for key in ["oxplow.high_complexity_fns", "oxplow.todos"] {
            let spec = svc.fact_store.get_spec(key).await.unwrap();
            if let Some(spec) = spec {
                assert_eq!(spec.language, None, "{key} is language-agnostic");
            }
        }
    }

    #[tokio::test]
    async fn duplicate_lines_is_a_sum_over_its_facts() {
        let (svc, _dir) = fixture().await;
        svc.metrics.seed_catalog().await;
        let spec = svc
            .fact_store
            .get_spec("oxplow.duplicate_lines")
            .await
            .unwrap()
            .expect("seeded");
        assert_eq!(
            (spec.source_measure.as_deref(), spec.aggregation.as_str()),
            (Some("oxplow.duplicate_lines"), "sum")
        );
        assert_eq!(spec.direction, "lower-better");
    }

    #[tokio::test]
    async fn entity_metrics_seed_read_live_and_capture_state() {
        let (svc, dir) = fixture().await;
        svc.db
            .transaction(|c| {
                c.execute_batch(
                    "INSERT INTO task (thread_id, title, status, priority, created_by, created_at, updated_at, completed_at) VALUES
                       ((SELECT min(id) FROM threads), 'a', 'done', 'high', 'agent', '2026-09-01', '2026-09-01', '2026-09-21T09:00:00Z'),
                       ((SELECT min(id) FROM threads), 'b', 'done', 'low', 'agent', '2026-09-01', '2026-09-01', '2026-09-23T09:00:00Z'),
                       ((SELECT min(id) FROM threads), 'c', 'ready', 'high', 'agent', '2026-09-01', '2026-09-01', NULL),
                       ((SELECT min(id) FROM threads), 'd', 'blocked', 'low', 'agent', '2026-09-01', '2026-09-01', NULL),
                       ((SELECT min(id) FROM threads), 'e', 'ready', 'high', 'agent', '2026-09-01', '2026-09-01', NULL);",
                )
                .map_err(|e| oxplow_domain::DomainError::Invalid(e.to_string()))
            })
            .await
            .unwrap();
        // A project entity metric with a typo'd column never reaches the catalog.
        std::fs::write(
            oxplow_config::config_path(dir.path()),
            "metrics:\n  - key: repo.bad\n    entity: v_task\n    where: \"no_such_col = 1\"\n",
        )
        .unwrap();
        svc.reload_config_from_disk().unwrap();
        svc.metrics.seed_catalog().await;
        let f = &svc.fact_store;
        assert!(f.get_spec("repo.bad").await.unwrap().is_none());

        // An event metric: computed live, daily, sliceable by its entity dims.
        let done = f.get_spec("work.tasks_completed").await.unwrap().unwrap();
        assert_eq!(done.source_measure, None);
        assert_eq!(done.display_kind, "event");
        assert_eq!(
            done.sliceable_dims_json.as_deref(),
            Some(r#"["work.priority"]"#)
        );
        let e = &svc.metric_engine;
        let series = e
            .series_for_spec_in_stream(&done, None, None, None)
            .await
            .unwrap();
        assert_eq!(
            series.iter().map(|p| p.value).collect::<Vec<_>>(),
            vec![1.0, 0.0, 1.0]
        );
        assert_eq!(
            e.headline_from_series(&done, &series).await.unwrap(),
            Some(2.0)
        );
        let by_prio = e
            .series_for_spec_in_stream(&done, Some("work.priority"), None, None)
            .await
            .unwrap();
        assert_eq!(by_prio.len(), 2);
        assert!(e
            .series_for_spec_in_stream(&done, Some("oxplow.package"), None, None)
            .await
            .is_err());

        // A state metric: captured as a fact, so its series is its history.
        let open = f.get_spec("work.open_tasks").await.unwrap().unwrap();
        assert_eq!(open.source_measure.as_deref(), Some("work.open_tasks"));
        assert_eq!(svc.metrics.capture_entity_states(true).await, 1);
        // Unchanged since the last capture: nothing new is written — not even
        // by a fresh process, which reads the last stored value.
        assert_eq!(svc.metrics.capture_entity_states(true).await, 0);
        svc.metrics.entity_captures.lock().unwrap().clear();
        assert_eq!(svc.metrics.capture_entity_states(true).await, 0);
        let hist = e
            .series_for_spec_in_stream(&open, None, None, None)
            .await
            .unwrap();
        assert_eq!(hist.iter().map(|p| p.value).collect::<Vec<_>>(), vec![3.0]);
        assert_eq!(e.headline_for_spec(&open).await.unwrap(), Some(3.0));
        // Grouped reads are live.
        let mut rows: Vec<(Option<String>, f64)> = e
            .series_for_spec_in_stream(&open, Some("work.priority"), None, None)
            .await
            .unwrap()
            .into_iter()
            .map(|p| (p.group, p.value))
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            rows,
            vec![
                (Some("high".to_string()), 2.0),
                (Some("low".to_string()), 1.0)
            ]
        );
        // Entity metrics are in the catalog like any other.
        let catalog = svc.metrics.catalog().await;
        assert!(catalog
            .iter()
            .any(|c| c.key == "work.open_tasks" && c.enabled));
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
            (2, 1),
            "the project's measure and oxplow-analytics' (bundled, on), one project dimension"
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
    async fn seed_catalog_prunes_project_rows_removed_from_config() {
        // tsk61: a project metric/measure deleted from project.yaml entirely
        // (not merely `enabled: false`) must not linger as a zombie catalog
        // row — four forever-blank gauges did exactly that for a week.
        let (svc, dir) = fixture().await;
        // Zombies: a project measure + spec that no config declares.
        svc.fact_store
            .upsert_measure(oxplow_db::NewMeasure {
                scope: "project".into(),
                ..oxplow_db::NewMeasure::new("repo.zombie", "Zombie")
            })
            .await
            .unwrap();
        let mut spec =
            oxplow_db::NewMetricSpec::base("repo.zombie", "Zombie", "repo.zombie", "sum");
        spec.scope = "project".into();
        svc.fact_store.upsert_spec(spec).await.unwrap();
        // A DECLARED project measure+metric that must survive.
        std::fs::write(
            oxplow_config::config_path(dir.path()),
            "measures:\n  - key: repo.kept\n    unit: count\n    \
             temporalSemantics: semi-additive\nmetrics:\n  - key: repo.kept\n    \
             title: Kept\n    sourceMeasure: repo.kept\n    aggregation: sum\n",
        )
        .unwrap();
        svc.reload_config_from_disk().unwrap();

        svc.metrics.seed_catalog().await;

        assert!(
            svc.fact_store
                .get_measure("repo.zombie")
                .await
                .unwrap()
                .is_none(),
            "undeclared project measure pruned"
        );
        assert!(
            svc.fact_store
                .get_spec("repo.zombie")
                .await
                .unwrap()
                .is_none(),
            "undeclared project spec pruned"
        );
        assert!(
            svc.fact_store
                .get_measure("repo.kept")
                .await
                .unwrap()
                .is_some(),
            "declared project measure survives"
        );
        assert!(
            svc.fact_store
                .get_spec("repo.kept")
                .await
                .unwrap()
                .is_some(),
            "declared project spec survives"
        );
        // Built-ins are never touched by project reconciliation.
        assert!(svc
            .fact_store
            .get_measure("oxplow.complexity")
            .await
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn an_unimplemented_aggregation_cannot_reach_the_catalog() {
        // tsk108. `p95`/`count_distinct` are reserved in the V44 CHECK's
        // vocabulary but the engine can't compute them, and a spec that seeds
        // anyway opens the COLLECTION gate (`measure_has_active_spec`) for a
        // metric that can never render. Two fences keep that impossible:
        // config validation rejects the string with the allowed set (pinned
        // here — the review assumed this fence didn't exist), and
        // `seed_catalog`'s computable() guard prunes rather than seeds if an
        // uncomputable spec ever arrives another way (a typo'd BUILT-IN list,
        // which no config validation sees).
        let (svc, dir) = fixture().await;
        std::fs::write(
            oxplow_config::config_path(dir.path()),
            "metrics:\n  - key: repo.latency_p95\n    title: \"p95\"\n    \
             sourceMeasure: acme.latency\n    aggregation: p95\n",
        )
        .unwrap();
        let err = svc.reload_config_from_disk().unwrap_err();
        assert!(
            err.to_string().contains("aggregation must be one of"),
            "config names the allowed vocabulary, got: {err}"
        );
        // The bad config never loaded; seeding runs with the prior (empty)
        // config and nothing reaches the catalog or the gate.
        svc.metrics.seed_catalog().await;
        assert!(
            svc.fact_store
                .get_spec("repo.latency_p95")
                .await
                .unwrap()
                .is_none(),
            "the rejected spec must not exist in the catalog"
        );
        assert!(
            !svc.fact_store
                .measure_has_active_spec("acme.latency")
                .await
                .unwrap(),
            "and the collection gate stays closed"
        );
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
            .run_collector_by_key("built-in", "oxplow.rust.unsafe_blocks", None, "human")
            .await
            .unwrap();
        assert_eq!(count, 0, "empty snapshot → no facts, runs without error");
    }

    #[tokio::test]
    async fn an_extension_contributes_measures_metrics_and_gauges() {
        let (svc, dir) = fixture().await;
        let ext = dir.path().join("oxplow/extensions/acme");
        std::fs::create_dir_all(ext.join("collectors")).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "name: acme\n\
             measures:\n  - { key: acme.todo, title: TODOs }\n\
             metrics:\n  - { key: acme.todos, title: TODOs, sourceMeasure: acme.todo, aggregation: sum }\n\
             collectors:\n  - { id: acme.todo_scan, runtime: starlark, entry: collectors/todo.star, facts: [acme.todo] }\n",
        )
        .unwrap();
        std::fs::write(
            ext.join("collectors/todo.star"),
            "def transform(input):\n    return {\"facts\": [{\"measure\": \"acme.todo\", \"value\": 3}]}\n",
        )
        .unwrap();

        svc.metrics.seed_catalog().await;
        let spec = svc
            .fact_store
            .get_spec("acme.todos")
            .await
            .unwrap()
            .expect("seeded");
        assert_eq!(spec.scope, "extension:acme");
        let measure = svc
            .fact_store
            .get_measure("acme.todo")
            .await
            .unwrap()
            .expect("measure");
        assert_eq!(measure.scope, "extension:acme");
        let entry = svc
            .metrics
            .catalog()
            .await
            .into_iter()
            .find(|e| e.key == "acme.todos")
            .expect("in the catalog");
        assert!(entry.enabled);

        // Its gauge runs its own script, read through the extension.
        let n = svc
            .metrics
            .run_collector_by_key("acme", "acme.todo_scan", None, "human")
            .await
            .unwrap();
        assert_eq!(n, 1);

        // Disabling the extension takes its metric out of the catalog; the
        // measure (and its facts) stay for when it's turned back on.
        std::fs::create_dir_all(dir.path().join(".oxplow")).unwrap();
        std::fs::write(
            dir.path().join(".oxplow/project.yaml"),
            "extensions:\n  disabled: [acme]\n",
        )
        .unwrap();
        svc.reload_config_from_disk().unwrap();
        svc.metrics.seed_catalog().await;
        assert!(svc
            .fact_store
            .get_spec("acme.todos")
            .await
            .unwrap()
            .is_none());
        assert!(svc
            .fact_store
            .get_measure("acme.todo")
            .await
            .unwrap()
            .is_some());
        assert!(svc
            .metrics
            .run_collector_by_key("acme", "acme.todo_scan", None, "human")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn an_extension_contributes_dimensions_that_slice_its_entity_metrics() {
        let (svc, dir) = fixture().await;
        let ext = dir.path().join("oxplow/extensions/acme");
        std::fs::create_dir_all(&ext).unwrap();
        std::fs::write(
            ext.join("extension.yaml"),
            "name: acme\n\
             dimensions:\n  - { key: acme.status, label: Status, entity: v_task, expr: e.status }\n\
             metrics:\n  - { key: acme.tasks, title: Tasks, entity: v_task }\n",
        )
        .unwrap();
        svc.metrics.seed_catalog().await;
        let dims = svc.fact_store.list_dimensions().await.unwrap();
        let d = dims
            .iter()
            .find(|d| d.key == "acme.status")
            .expect("seeded");
        assert_eq!(d.scope, "extension:acme");
        let spec = svc
            .fact_store
            .get_spec("acme.tasks")
            .await
            .unwrap()
            .unwrap();
        assert!(
            spec.sliceable_dims_json
                .unwrap_or_default()
                .contains("acme.status"),
            "an extension entity metric picks up its entity dimension"
        );
        // Disabling the extension removes its dimension too.
        std::fs::create_dir_all(dir.path().join(".oxplow")).unwrap();
        std::fs::write(
            dir.path().join(".oxplow/project.yaml"),
            "extensions:\n  disabled: [acme]\n",
        )
        .unwrap();
        svc.reload_config_from_disk().unwrap();
        svc.metrics.seed_catalog().await;
        assert!(!svc
            .fact_store
            .list_dimensions()
            .await
            .unwrap()
            .iter()
            .any(|d| d.key == "acme.status"));
    }

    #[tokio::test]
    async fn a_project_exec_gauge_runs_only_once_a_person_approved_it() {
        use std::os::unix::fs::PermissionsExt;
        let (svc, dir) = fixture().await;
        let script = dir.path().join("tools/count.sh");
        std::fs::create_dir_all(script.parent().unwrap()).unwrap();
        std::fs::write(
            &script,
            "#!/bin/sh\necho '{\"facts\":[{\"measure\":\"repo.n\",\"value\":2}]}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(
            oxplow_config::config_path(dir.path()),
            "measures:\n  - { key: repo.n, title: N }\ncollectors:\n  - { id: repo.count, runtime: exec, entry: tools/count.sh, facts: [repo.n] }\n",
        )
        .unwrap();
        svc.reload_config_from_disk().unwrap();
        svc.metrics.seed_catalog().await;
        let err = svc
            .metrics
            .run_collector_by_key("project", "repo.count", None, "human")
            .await
            .unwrap_err();
        assert!(err.contains("approval"), "{err}");
        let cfg = svc.config.read().unwrap().clone();
        crate::exec_consent::approve_program(
            &svc.approvals,
            dir.path(),
            &cfg,
            &[],
            crate::exec_consent::ProgramKind::Collector,
            "repo.count",
            &crate::exec_consent::version_of(
                &svc.approvals,
                dir.path(),
                &cfg,
                crate::exec_consent::ProgramKind::Collector,
                "repo.count",
            ),
        )
        .unwrap();
        assert_eq!(
            svc.metrics
                .run_collector_by_key("project", "repo.count", None, "human")
                .await
                .unwrap(),
            1
        );
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
        enable(&svc, &["oxplow.rust.unsafe_blocks".into()], true).await;
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
        enable(&svc, &["oxplow.rust.unsafe_blocks".into()], false).await;
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
        enable(&svc, &["agent.tokens.total".into()], false).await;
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
        enable(&svc, &["agent.tokens.total".into()], true).await;
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
        enable(&svc, &keys, false).await;

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
            enable(&svc, &[k.to_string()], false).await;
        }
        assert!(
            !svc.fact_store
                .measure_has_active_spec("oxplow.tokens")
                .await
                .unwrap(),
            "all consumers disabled → gate closed → producer stops collecting"
        );
    }

    /// A metric scaffold writes nothing: the agent writes the returned
    /// script and pastes the entries with its own tools, under the write
    /// guard and filing (tsk391). Done that way, the metric seeds.
    #[tokio::test]
    async fn metric_scaffold_is_a_template_that_seeds_once_written() {
        let (svc, dir) = fixture().await;
        let config_before = std::fs::read_to_string(oxplow_config::config_path(dir.path())).ok();

        let t = svc
            .metrics
            .metric_scaffold(
                "acme.todo_density",
                Some("TODO density".to_string()),
                Some("rust".to_string()),
                Some("**/*.rs".to_string()),
            )
            .unwrap();
        assert_eq!(t.script_path, "oxplow/collectors/acme_todo_density.star");
        assert!(t.script.contains("def transform(input):"), "{}", t.script);
        assert!(t.script.contains("files(\"**/*.rs\")"), "{}", t.script);
        assert!(t.script.contains("acme.todo_density.count"), "{}", t.script);
        assert!(
            t.project_yaml.contains("acme_todo_density.star"),
            "{}",
            t.project_yaml
        );
        assert!(!dir.path().join(&t.script_path).exists(), "nothing written");
        assert_eq!(
            std::fs::read_to_string(oxplow_config::config_path(dir.path())).ok(),
            config_before,
            "project.yaml untouched"
        );

        // What the agent does with it.
        let script = dir.path().join(&t.script_path);
        std::fs::create_dir_all(script.parent().unwrap()).unwrap();
        std::fs::write(&script, &t.script).unwrap();
        std::fs::create_dir_all(dir.path().join(".oxplow")).unwrap();
        std::fs::write(oxplow_config::config_path(dir.path()), &t.project_yaml).unwrap();
        svc.reload_config_from_disk().unwrap();
        svc.metrics.seed_catalog().await;
        let spec = svc
            .fact_store
            .get_spec("acme.todo_density")
            .await
            .unwrap()
            .expect("scaffolded spec seeds once written");
        assert_eq!(spec.display_kind, "gauge");
        assert_eq!(spec.scope, "project");

        // An existing key and the reserved namespace are refused.
        assert!(svc
            .metrics
            .metric_scaffold("acme.todo_density", None, None, None)
            .is_err());
        assert!(svc
            .metrics
            .metric_scaffold("oxplow.nope", None, None, None)
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
}
