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

use oxplow_collect_plugin::{Collector, CollectorInput, CollectorKind, GaugeHost};
use oxplow_config::{
    global_config_dir, load_global_metric_entries, resolve_metrics, MetricComputeConfig,
    OxplowConfig, ResolvedMetric,
};
use oxplow_db::{
    NewMetricDefinition, NewMetricRun, NewMetricSample, SnapshotStorage, SqliteMetricStore,
    SqliteSnapshotStore, SqliteTaskEffortStore, SqliteThreadStore, TaskEffortStore,
};
use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{DomainError, EffortId, StreamId, ThreadId};

use crate::blob_store::BlobStore;
use crate::events::{EventBus, OxplowEvent, SnapshotSourceKind};
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
        }
    }

    /// The active, resolved metrics for this project (built-in ∪ global ∪
    /// project, precedence project > global > built-in). Built-ins are empty
    /// until the bundled catalog lands (tsk218).
    fn resolved_metrics(&self) -> Vec<ResolvedMetric> {
        let project = self
            .config
            .read()
            .map(|c| c.metrics.clone())
            .unwrap_or_default();
        let global = global_config_dir()
            .map(|d| load_global_metric_entries(&d))
            .unwrap_or_default();
        resolve_metrics(&[], &global, &project)
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

    /// Event loop: seed once, then reseed on `ConfigChanged` and run on-snapshot
    /// gauges when a snapshot batch lands. Spawned at boot (see `boot.rs`).
    pub async fn run(self, mut rx: tokio::sync::broadcast::Receiver<OxplowEvent>) {
        self.seed_definitions().await;
        loop {
            match rx.recv().await {
                Ok(OxplowEvent::ConfigChanged) => {
                    self.seed_definitions().await;
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
        let files = self.build_file_map(snapshot_id).await;
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
        let files = match snapshot_id {
            Some(sid) => self.build_file_map(sid).await,
            None => HashMap::new(),
        };
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
        let files = match snapshot_id {
            Some(sid) => self.build_file_map(sid).await,
            None => HashMap::new(),
        };
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
        files: HashMap<String, String>,
    ) -> usize {
        let collector = match compute_to_collector(metric, &self.project_dir) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(key = %metric.key, error = %e, "gauge metric: bad compute");
                return 0;
            }
        };
        let source = gauge_source(metric, &collector);
        // The report-derived content (if any); tree-derived gauges ignore it.
        let content = match &metric.compute.report {
            Some(rel) => std::fs::read_to_string(self.project_dir.join(rel)).unwrap_or_default(),
            None => String::new(),
        };
        let host = GaugeHost::new(files);
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
        let samples = match report {
            oxplow_collect_plugin::CollectorOutput::Gauge(r) => r.samples,
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
            let run_id = self.metrics.record_run(run).await?;

            let mut count = 0;
            for sample in &samples {
                let (subject_kind, subject_ref) = resolve_subject(&sample.subject, ctx);
                let mut s = NewMetricSample::observed(
                    metric_id,
                    ctx.stream_val,
                    sample.value,
                    source.clone(),
                );
                s.run_id = Some(run_id);
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
                self.metrics.record_sample(s).await?;
                count += 1;
            }
            Ok::<usize, DomainError>(count)
        }
        .await;

        match result {
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
        let count = svc.metrics.run_one_gauge(&metric, &ctx, files).await;
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
    async fn seed_definitions_upserts_configured_metrics() {
        let (svc, dir) = fixture().await;
        std::fs::write(
            dir.path().join("oxplow.yaml"),
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
