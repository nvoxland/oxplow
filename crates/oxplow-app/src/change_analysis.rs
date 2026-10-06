//! The change-analysis producer: for one change (a commit, an effort, or
//! the working tree) it diffs the two endpoints, analyzes the changed files
//! (function metrics, churn, imports, zones, co-change) and stores the rows
//! behind `v_change*`, which the analytics lenses read. See
//! `.context/semantic-layer.md` → "Change analysis".

use std::collections::{BTreeMap, BTreeSet};

use oxplow_code_deps::ZoneRules;
use oxplow_db::{ChangeFileRow, ChangeFunctionRow, ChangeImportRow, ChangeResults};

use crate::code_analysis::{
    analyze_files, AnalyzeFileSpec, AnalyzeFunctionsResult, AnalyzedFunction,
};
use oxplow_domain::vcs::Revision;

/// Most files a change analyzes function-by-function (matching the old UI).
pub const MAX_ANALYZED_FILES: usize = 200;

/// One changed file, with its contents on each side (`None` = absent there).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ChangedFile {
    pub path: String,
    pub status: String,
    pub additions: i64,
    pub deletions: i64,
    pub base: Option<String>,
    pub head: Option<String>,
}

/// True if `path` looks like a test file by convention.
pub fn is_test_path(path: &str) -> bool {
    let path = path.replace('\\', "/");
    let file = path.rsplit('/').next().unwrap_or(&path);
    // `.test.<ext>` / `.spec.<ext>` with an alphabetic extension.
    let marked = |marker: &str| {
        file.rfind('.').is_some_and(|dot| {
            let ext = &file[dot + 1..];
            !ext.is_empty()
                && ext.chars().all(|c| c.is_ascii_alphabetic())
                && file[..dot].ends_with(marker)
        })
    };
    let in_test_dir = ["test/", "tests/"]
        .iter()
        .any(|d| path.starts_with(d) || path.contains(&format!("/{d}")));
    marked(".test")
        || marked(".spec")
        || in_test_dir
        || (file.starts_with("test_") && (file.ends_with(".py") || file.ends_with(".rs")))
        || file.ends_with("_test.go")
        || file.ends_with("_test.clj")
        || file.ends_with("_test.cljc")
}

/// `name` starts with `prefix` followed by an uppercase letter, `_`, or nothing.
fn prefixed(name: &str, prefix: &str) -> bool {
    name.strip_prefix(prefix).is_some_and(|rest| {
        rest.chars()
            .next()
            .is_none_or(|c| c.is_ascii_uppercase() || c == '_')
    })
}

fn is_test_container(name: &str) -> bool {
    name == "tests"
        || name == "test"
        || name.ends_with("Test")
        || name.ends_with("Tests")
        || prefixed(name, "Test")
        || name.ends_with("-test")
}

/// True if a function is a test: in a test file, named like one, or inside
/// a test module/class (`mod tests`, `FooTest`, `foo.bar-test`).
pub fn is_test_function(path: &str, name: &str, container: &[String]) -> bool {
    is_test_path(path)
        || name.starts_with("test_")
        || name.strip_prefix("test").is_some_and(|rest| {
            rest.chars()
                .next()
                .is_some_and(|c| c.is_ascii_uppercase() || c == '_')
        })
        || prefixed(name, "Test")
        || prefixed(name, "Benchmark")
        || prefixed(name, "Example")
        || container.iter().any(|c| is_test_container(c))
}

fn qualified(f: &AnalyzedFunction) -> String {
    f.container_path
        .iter()
        .chain([&f.name])
        .cloned()
        .collect::<Vec<_>>()
        .join("::")
}

/// One function's base and head versions (either may be absent).
type BaseHead<'a> = (Option<&'a AnalyzedFunction>, Option<&'a AnalyzedFunction>);

/// Build the stored rows from the diff and the function analysis.
pub fn build_results(
    files: &[ChangedFile],
    analysis: &AnalyzeFunctionsResult,
    zones: &ZoneRules,
) -> ChangeResults {
    // (path, qualified name) → (base, head); the first of a duplicated name wins.
    let mut index: BTreeMap<(String, String), BaseHead<'_>> = BTreeMap::new();
    for side in &analysis.sides {
        for f in &side.functions {
            let slot = index.entry((side.path.clone(), qualified(f))).or_default();
            match side.side.as_str() {
                "base" if slot.0.is_none() => slot.0 = Some(f),
                "head" if slot.1.is_none() => slot.1 = Some(f),
                _ => {}
            }
        }
    }
    let churn_total: u32 = analysis
        .churn
        .iter()
        .flat_map(|c| &c.functions)
        .map(|f| f.added_lines + f.deleted_lines)
        .sum();
    let mut churn = BTreeMap::new();
    for c in &analysis.churn {
        for f in &c.functions {
            let key = f
                .container_path
                .iter()
                .chain([&f.name])
                .cloned()
                .collect::<Vec<_>>()
                .join("::");
            churn.insert((c.path.clone(), key), f);
        }
    }

    let mut functions = Vec::new();
    for ((path, key), (before, after)) in &index {
        let (status, sig, body) = match (before, after) {
            (None, Some(_)) => ("added", false, false),
            (Some(_), None) => ("deleted", false, false),
            (Some(b), Some(a)) => {
                let sig = b.parameter_count != a.parameter_count;
                let body = a.complexity != b.complexity || a.length != b.length;
                if !sig && !body {
                    continue;
                }
                ("modified", sig, body)
            }
            (None, None) => continue,
        };
        let f = after.or(*before).expect("one side exists");
        let c = churn.get(&(path.clone(), key.clone()));
        functions.push(ChangeFunctionRow {
            path: path.clone(),
            container: f.container_path.join("::"),
            name: f.name.clone(),
            status: status.into(),
            signature_changed: sig,
            body_changed: body,
            start_line: f.start_line as i64,
            visibility: f.visibility.clone(),
            is_test: is_test_function(path, &f.name, &f.container_path),
            complexity: Some(f.complexity),
            length: Some(f.length as i64),
            params_before: before.map(|b| b.parameter_count as i64),
            params_after: after.map(|a| a.parameter_count as i64),
            complexity_delta: before.zip(*after).map(|(b, a)| a.complexity - b.complexity),
            length_delta: before
                .zip(*after)
                .map(|(b, a)| a.length as i64 - b.length as i64),
            added_lines: c.map(|c| c.added_lines as i64),
            deleted_lines: c.map(|c| c.deleted_lines as i64),
            modified_lines: c.map(|c| c.modified_lines as i64),
            churn_share: c.map(|c| {
                if churn_total == 0 {
                    0.0
                } else {
                    (c.added_lines + c.deleted_lines) as f64 / churn_total as f64
                }
            }),
        });
    }

    let files = file_rows(files, zones);

    let mut imports = Vec::new();
    for d in &analysis.import_deltas {
        let cross: BTreeSet<(String, u32)> = d
            .cross_zone_added
            .iter()
            .map(|e| (e.edge.module.clone(), e.edge.start_line))
            .collect();
        for (direction, edges) in [("added", &d.added), ("removed", &d.removed)] {
            for e in edges {
                imports.push(ChangeImportRow {
                    path: d.path.clone(),
                    module: e.edge.module.clone(),
                    direction: direction.into(),
                    start_line: Some(e.edge.start_line as i64),
                    from_zone: Some(e.from_zone.clone()),
                    to_zone: e.to_zone.clone(),
                    cross_zone: direction == "added"
                        && cross.contains(&(e.edge.module.clone(), e.edge.start_line)),
                });
            }
        }
    }

    ChangeResults {
        files,
        functions,
        imports,
        test_files: Vec::new(),
    }
}

/// The stored row of each changed file: its status and line counts, its
/// zone and whether it's a test — what both stages write.
fn file_rows(files: &[ChangedFile], zones: &ZoneRules) -> Vec<ChangeFileRow> {
    files
        .iter()
        .map(|f| ChangeFileRow {
            path: f.path.clone(),
            status: f.status.clone(),
            additions: f.additions,
            deletions: f.deletions,
            zone: Some(zones.classify(&f.path)),
            is_test: is_test_path(&f.path),
        })
        .collect()
}

/// Test functions, assertions and skip markers on each side of every
/// analyzed file that is a test file or has tests on either side (Rust's
/// inline `mod tests` counts). Files with no tests on either side are left
/// out. `paths` lines up with the two content lists.
pub fn test_file_rows(
    paths: &[String],
    base: &[Option<String>],
    head: &[Option<String>],
    analysis: &AnalyzeFunctionsResult,
) -> Vec<oxplow_db::ChangeTestFileRow> {
    let tests_on = |path: &str, side: &str| -> i64 {
        analysis
            .sides
            .iter()
            .filter(|s| s.path == path && s.side == side)
            .flat_map(|s| s.functions.iter())
            .filter(|f| is_test_function(path, &f.name, &f.container_path))
            .count() as i64
    };
    let signals = |content: &Option<String>| {
        content
            .as_deref()
            .map(crate::test_signals::count)
            .unwrap_or_default()
    };
    paths
        .iter()
        .zip(base.iter().zip(head.iter()))
        .filter_map(|(path, (b, h))| {
            let (tests_before, tests_after) = (tests_on(path, "base"), tests_on(path, "head"));
            if !is_test_path(path) && tests_before == 0 && tests_after == 0 {
                return None;
            }
            let (before, after) = (signals(b), signals(h));
            Some(oxplow_db::ChangeTestFileRow {
                path: path.clone(),
                tests_before,
                tests_after,
                assertions_before: before.assertions,
                assertions_after: after.assertions,
                skips_before: before.skips,
                skips_after: after.skips,
            })
        })
        .collect()
}

/// Stored rows for the surprising files (normal ones are left out).
/// What to analyze.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ChangeTarget {
    /// The stream's uncommitted work: HEAD → the working tree.
    Working {
        #[serde(rename = "streamId")]
        stream_id: String,
    },
    /// One commit against its first parent. `stream_id` picks the repo
    /// (default: the primary stream's).
    Commit {
        sha: String,
        #[serde(rename = "streamId", default)]
        stream_id: Option<String>,
    },
    /// An effort: its start snapshot → its end snapshot (or the working
    /// tree while it's open).
    Effort {
        #[serde(rename = "effortId")]
        effort_id: String,
    },
    /// An agent turn: its start snapshot → its end snapshot, "what changed
    /// this turn" (P2.10). A turn still running has no end: `NotFound`.
    Turn {
        #[serde(rename = "turnId")]
        turn_id: String,
    },
}

/// Which changes are being computed and the duplicate-scan queue.
#[derive(Default)]
pub struct ChangeAnalyzer {
    state: std::sync::Mutex<AnalyzerState>,
    /// Duplicate scans: one worker per change at a time.
    dup_queue: DupQueue,
}

/// One duplicate scan to run for a change.
pub(crate) struct DupJob {
    /// The analysis it belongs to (`change.events_to`): its findings are
    /// stored only if that's still the change's latest.
    events_to: i64,
    root: std::path::PathBuf,
    revision: Revision,
    changed: Vec<String>,
}

/// Coalesces duplicate scans per change (tsk364): a whole-tree parse per
/// agent edit would pile up, so at most one runs per change, and the
/// newest request replaces any queued one.
#[derive(Default)]
pub(crate) struct DupQueue {
    inner: std::sync::Mutex<DupQueueState>,
}

#[derive(Default)]
struct DupQueueState {
    active: std::collections::HashSet<i64>,
    queued: std::collections::HashMap<i64, DupJob>,
}

impl DupQueue {
    /// Queue `job` for `change`. True when the caller must start a worker
    /// (none is running for it).
    fn submit(&self, change: i64, job: DupJob) -> bool {
        let mut st = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        st.queued.insert(change, job);
        st.active.insert(change)
    }

    /// The next job for `change`'s worker; `None` ends the worker.
    fn next(&self, change: i64) -> Option<DupJob> {
        let mut st = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let job = st.queued.remove(&change);
        if job.is_none() {
            st.active.remove(&change);
        }
        job
    }
}

/// Marks a change in flight; removed however the computation ends
/// (an error return, a dropped future), so it can't stick (tsk364).
struct RunningGuard<'a> {
    state: &'a std::sync::Mutex<AnalyzerState>,
    id: i64,
}

impl Drop for RunningGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut st) = self.state.lock() {
            st.running.remove(&self.id);
        }
    }
}

#[derive(Default)]
struct AnalyzerState {
    running: std::collections::HashSet<i64>,
}

/// Hold change `id` as being analyzed, as a long deep analysis would — a
/// test's stand-in for one still running.
#[cfg(test)]
pub(crate) fn hold_running_for_tests(svc: &crate::Services, id: i64) {
    svc.change_analyzer.state.lock().unwrap().running.insert(id);
}

/// Analyze `target` if it hasn't been (or its head moved — an effort
/// closed), store the results, and return the change. A working tree's
/// and an open effort's analysis is kept current by the `change.analyze`
/// consumer ([`refresh_change`]); this reads it, computing it the first
/// time. While another call is computing the same change this returns it
/// with status `running`; its rows' commit announces the results
/// (`ModelsChanged` on `v_change`).
pub async fn ensure_change(
    svc: &crate::Services,
    target: ChangeTarget,
) -> Result<oxplow_db::ChangeRow, oxplow_domain::DomainError> {
    analyze(svc, target, false).await
}

/// Recompute `target` now, however fresh it is (the `change.analyze`
/// consumer, as its stream moves). `Busy` while another computation of it
/// runs, so the consumer retries rather than losing the move.
pub async fn refresh_change(
    svc: &crate::Services,
    target: ChangeTarget,
) -> Result<oxplow_db::ChangeRow, oxplow_domain::DomainError> {
    analyze(svc, target, true).await
}

/// What `target` compares: its stream, kind and key, and its two
/// revisions.
/// Stage one (tsk1095): list `target`'s changed files — status and line
/// counts — and, for a working tree, git's operation in progress and how
/// many files conflict, and store them alone, stamped with what they saw.
/// Cheap: no file is parsed, so the `change.analyze` consumer runs it on
/// every move and the list (the Uncommitted panel's) stays current; the
/// deep analysis keeps its own freshness (`events_to`).
pub async fn refresh_files(
    svc: &crate::Services,
    target: ChangeTarget,
) -> Result<oxplow_db::ChangeRow, oxplow_domain::DomainError> {
    use oxplow_domain::DomainError;
    let (stream, kind, key, base, head) = resolve(svc, target).await?;
    let (row, _) = svc
        .change_store
        .get_or_create(stream.value(), kind, &key, base.as_ref(), &head)
        .await?;
    let root = svc.worktrees.resolve(Some(&stream.to_string())).await;
    let events_to = events_to(svc).await?;
    let entries = svc.trees.diff(&root, base.as_ref(), &head).await?;
    let files: Vec<ChangedFile> = entries
        .into_iter()
        .map(|e| ChangedFile {
            path: e.path,
            status: e.status.as_str().to_string(),
            additions: e.additions as i64,
            deletions: e.deletions as i64,
            base: None,
            head: None,
        })
        .collect();
    let rows = {
        let cfg = svc.config.read().unwrap_or_else(|e| e.into_inner());
        file_rows(&files, &ZoneRules::from_config(&cfg.zones))
    };
    let (conflicted, in_progress) = if kind == "working" {
        let status = svc.vcs.status(&root).await?;
        let conflicted = status
            .entries
            .iter()
            .filter(|e| e.status == oxplow_domain::vcs::FileStatus::Conflicted)
            .count() as i64;
        let op = status
            .in_progress
            .and_then(|op| serde_json::to_value(op).ok())
            .and_then(|v| v.as_str().map(str::to_string));
        (Some(conflicted), op)
    } else {
        (None, None)
    };
    svc.change_store
        .store_files(row.id, rows, events_to, conflicted, in_progress)
        .await?;
    svc.change_store
        .get(row.id)
        .await?
        .ok_or(DomainError::NotFound)
}

async fn resolve(
    svc: &crate::Services,
    target: ChangeTarget,
) -> Result<
    (
        oxplow_domain::StreamId,
        &'static str,
        String,
        Option<Revision>,
        Revision,
    ),
    oxplow_domain::DomainError,
> {
    use oxplow_db::EffortStore as _;
    use oxplow_domain::stores::{StreamStore as _, ThreadStore as _};
    use oxplow_domain::DomainError;

    let invalid = |m: String| DomainError::Invalid(m);
    let primary_stream = || async {
        svc.stream_store
            .list()
            .await
            .ok()
            .and_then(|l| {
                l.into_iter()
                    .find(|s| s.kind == oxplow_domain::StreamKind::Primary)
            })
            .map(|s| s.id)
    };
    let vcs_rev = |rev: String| Revision::Vcs {
        kind: svc.vcs.rev_kind().into(),
        rev,
    };
    Ok(match target {
        ChangeTarget::Working { stream_id } => {
            let sid = oxplow_domain::StreamId::try_from_str(&stream_id)
                .ok_or_else(|| invalid(format!("not a stream id: {stream_id}")))?;
            let root = svc.worktrees.resolve(Some(&sid.to_string())).await;
            let head = svc.vcs.head(&root).await?.revision;
            (
                sid,
                "working",
                String::new(),
                head.map(vcs_rev),
                Revision::Working,
            )
        }
        ChangeTarget::Commit { sha, stream_id } => {
            let sid = match stream_id.as_deref() {
                Some(s) => oxplow_domain::StreamId::try_from_str(s)
                    .ok_or_else(|| invalid(format!("not a stream id: {s}")))?,
                None => primary_stream()
                    .await
                    .ok_or_else(|| invalid("no primary stream".into()))?,
            };
            let root = svc.worktrees.resolve(Some(&sid.to_string())).await;
            let full = svc.vcs.resolve(&root, &sha).await?;
            let parent = svc
                .vcs
                .revision(&root, &full)
                .await?
                .and_then(|r| r.info.parents.into_iter().next());
            (
                sid,
                "commit",
                full.clone(),
                parent.map(vcs_rev),
                vcs_rev(full),
            )
        }
        ChangeTarget::Turn { turn_id } => {
            use oxplow_domain::stores::AgentTurnStore as _;
            let tid = oxplow_domain::AgentTurnId::try_from_str(&turn_id)
                .ok_or_else(|| invalid(format!("not a turn id: {turn_id}")))?;
            let turn = svc
                .agent_turn_store
                .get(&tid)
                .await?
                .ok_or(DomainError::NotFound)?;
            let end = turn.snapshot_id.ok_or(DomainError::NotFound)?;
            let start = turn.start_snapshot_id.ok_or_else(|| {
                invalid(format!(
                    "turn {turn_id} began before the stream had a snapshot, so there's nothing to diff from"
                ))
            })?;
            let thread = svc
                .thread_store
                .get(&turn.thread_id)
                .await?
                .ok_or(DomainError::NotFound)?;
            (
                thread.stream_id,
                "turn",
                tid.value().to_string(),
                Some(Revision::Snapshot(start)),
                Revision::Snapshot(end),
            )
        }
        ChangeTarget::Effort { effort_id } => {
            let eid = oxplow_domain::EffortId::try_from_str(&effort_id)
                .ok_or_else(|| invalid(format!("not an effort id: {effort_id}")))?;
            let effort = svc
                .effort_store
                .get_effort(&eid)
                .await?
                .ok_or(DomainError::NotFound)?;
            let start = effort.start_snapshot_id.ok_or_else(|| {
                invalid(format!(
                    "effort {effort_id} has no start snapshot, so there's nothing to diff"
                ))
            })?;
            let thread = svc
                .thread_store
                .get(&effort.thread_id)
                .await?
                .ok_or(DomainError::NotFound)?;
            let head = match effort.end_snapshot_id {
                Some(end) if effort.ended_at.is_some() => Revision::Snapshot(end),
                _ => Revision::Working,
            };
            (
                thread.stream_id,
                "effort",
                eid.value().to_string(),
                Some(Revision::Snapshot(start)),
                head,
            )
        }
    })
}

async fn analyze(
    svc: &crate::Services,
    target: ChangeTarget,
    force: bool,
) -> Result<oxplow_db::ChangeRow, oxplow_domain::DomainError> {
    use oxplow_domain::DomainError;

    let invalid = |m: String| DomainError::Invalid(m);
    let (stream, kind, key, base, head) = resolve(svc, target).await?;
    let stream_val = stream.value();
    let (row, head_moved) = svc
        .change_store
        .get_or_create(stream_val, kind, &key, base.as_ref(), &head)
        .await?;
    {
        let mut st = svc
            .change_analyzer
            .state
            .lock()
            .map_err(|_| invalid("analyzer lock".into()))?;
        if st.running.contains(&row.id) {
            return if force {
                Err(DomainError::Busy(format!(
                    "change {} is being analyzed",
                    row.id
                )))
            } else {
                Ok(row)
            };
        }
        // Results of another head (an effort analyzed while open, now
        // closed) are stale however they were computed.
        if row.status == "done" && !head_moved && !force {
            return Ok(row);
        }
        st.running.insert(row.id);
    }
    let running = RunningGuard {
        state: &svc.change_analyzer.state,
        id: row.id,
    };
    svc.change_store.set_status(row.id, "running", None).await?;
    let root = svc.worktrees.resolve(Some(&stream.to_string())).await;
    // What the inputs had seen as it began, and the snapshot it's against.
    let events_to = events_to(svc).await?;
    let snapshot_id = match &head {
        Revision::Snapshot(id) => Some(*id),
        Revision::Working => {
            svc.snapshot_store
                .latest_snapshot_id_for_stream(stream)
                .await?
        }
        Revision::Vcs { .. } => None,
    };
    let dup_head = head.clone();
    let result = compute(svc, &root, base, head).await;
    drop(running);
    match result {
        Ok(results) => {
            let changed: Vec<String> = results.files.iter().map(|f| f.path.clone()).collect();
            svc.change_store
                .store_results(row.id, results, snapshot_id, events_to)
                .await?;
            spawn_duplicates(svc, row.id, events_to, &root, &dup_head, changed);
        }
        Err(e) => {
            svc.change_store
                .set_status(row.id, "failed", Some(e.clone()))
                .await?;
            return Err(invalid(format!("analyzing the change failed: {e}")));
        }
    }
    svc.change_store
        .get(row.id)
        .await?
        .ok_or(DomainError::NotFound)
}

/// The event log's highest seq now.
async fn events_to(svc: &crate::Services) -> Result<i64, oxplow_domain::DomainError> {
    svc.db
        .read(|c| {
            c.query_row("SELECT coalesce(max(seq), 0) FROM event_log", [], |r| {
                r.get(0)
            })
            .map_err(oxplow_db::map_sql_err)
        })
        .await
}

/// Find duplicated blocks between the changed files and anything else in
/// the tree at `head`, in the background (it parses the whole tree), then
/// store them if the analysis they belong to (`events_to`) is still the
/// change's latest. The scan is also recorded as a code-quality scan
/// ([`crate::duplication_scan`]).
fn spawn_duplicates(
    svc: &crate::Services,
    change_id: i64,
    events_to: i64,
    root: &std::path::Path,
    head: &Revision,
    changed: Vec<String>,
) {
    if changed.is_empty() {
        return;
    }
    let job = DupJob {
        events_to,
        root: root.to_path_buf(),
        revision: head.clone(),
        changed,
    };
    if !svc.change_analyzer.dup_queue.submit(change_id, job) {
        return; // A worker is running; it picks this up next.
    }
    let recorder = crate::duplication_scan::DuplicationRecorder::new(svc);
    let store = svc.change_store.clone();
    let analyzer = svc.change_analyzer.clone();
    tokio::spawn(async move {
        while let Some(job) = analyzer.dup_queue.next(change_id) {
            let scope = format!("change {change_id}");
            let findings = match recorder
                .record(job.root, job.revision, job.changed.clone(), scope)
                .await
            {
                Ok(f) => f,
                Err(error) => {
                    tracing::warn!(change_id, %error, "duplicate scan failed");
                    continue;
                }
            };
            let rows: Vec<oxplow_db::ChangeDuplicateRow> = findings
                .into_iter()
                .filter(|f| job.changed.contains(&f.path))
                .map(|f| {
                    let extra: serde_json::Value = f
                        .extra_json
                        .as_deref()
                        .and_then(|j| serde_json::from_str(j).ok())
                        .unwrap_or_default();
                    oxplow_db::ChangeDuplicateRow {
                        path: f.path,
                        start_line: f.start_line as i64,
                        end_line: f.end_line as i64,
                        lines: f.metric_value as i64,
                        peer_path: extra["peerPath"].as_str().unwrap_or_default().to_string(),
                        peer_start_line: extra["peerStartLine"].as_i64().unwrap_or_default(),
                        peer_end_line: extra["peerEndLine"].as_i64().unwrap_or_default(),
                    }
                })
                .collect();
            // A newer analysis of this change supersedes these findings.
            if let Err(error) = store.store_duplicates(change_id, rows, job.events_to).await {
                tracing::warn!(change_id, %error, "storing duplicates failed");
            }
        }
    });
}

/// Diff `base` → `head` in `root`, read the changed files' contents, and
/// analyze them.
async fn compute(
    svc: &crate::Services,
    root: &std::path::Path,
    base: Option<Revision>,
    head: Revision,
) -> Result<ChangeResults, String> {
    let zones = {
        let cfg = svc.config.read().unwrap_or_else(|e| e.into_inner());
        ZoneRules::from_config(&cfg.zones)
    };
    let entries = svc
        .trees
        .diff(root, base.as_ref(), &head)
        .await
        .map_err(|e| e.to_string())?;
    let analyzed: Vec<String> = entries
        .iter()
        .take(MAX_ANALYZED_FILES)
        .map(|e| e.path.clone())
        .collect();
    let text = |b: Option<Vec<u8>>| b.map(|b| String::from_utf8_lossy(&b).into_owned());
    let mut base_contents = Vec::with_capacity(analyzed.len());
    let mut head_contents = Vec::with_capacity(analyzed.len());
    for path in &analyzed {
        base_contents.push(match &base {
            Some(b) => text(
                svc.trees
                    .read_at(root, b, path)
                    .await
                    .map_err(|e| e.to_string())?,
            ),
            None => None,
        });
        head_contents.push(text(
            svc.trees
                .read_at(root, &head, path)
                .await
                .map_err(|e| e.to_string())?,
        ));
    }
    tokio::task::spawn_blocking(move || -> Result<ChangeResults, String> {
        let specs: Vec<AnalyzeFileSpec> = analyzed
            .iter()
            .zip(base_contents.iter().zip(head_contents.iter()))
            .map(|(path, (b, h))| AnalyzeFileSpec {
                path: path.clone(),
                base_content: b.clone(),
                head_content: h.clone(),
            })
            .collect();
        let analysis = analyze_files(specs, &zones);
        let files: Vec<ChangedFile> = entries
            .into_iter()
            .map(|e| ChangedFile {
                path: e.path,
                status: e.status.as_str().to_string(),
                additions: e.additions as i64,
                deletions: e.deletions as i64,
                base: None,
                head: None,
            })
            .collect();
        let mut results = build_results(&files, &analysis, &zones);
        results.test_files = test_file_rows(&analyzed, &base_contents, &head_contents, &analysis);
        Ok(results)
    })
    .await
    .map_err(|e| format!("analysis task: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_analysis::{AnalyzedFileChurn, AnalyzedFileSide, AnalyzedFunctionChurn};

    #[test]
    fn test_paths_by_convention() {
        for p in [
            "apps/desktop/src/foo.test.ts",
            "src/bar.spec.tsx",
            "packages/x/tests/helper.ts",
            "crates/foo/src/test_inner.rs",
            "internal/stuff/widget_test.go",
            "src/foo/bar_test.clj",
            "src/foo/bar_test.cljc",
            "test/x.py",
        ] {
            assert!(is_test_path(p), "{p}");
        }
        for p in [
            "src/foo.ts",
            "crates/oxplow-app/src/lib.rs",
            "apps/desktop/src/api.ts",
            "src/latest/x.rs",
            "src/contest.ts",
        ] {
            assert!(!is_test_path(p), "{p}");
        }
    }

    #[test]
    fn test_functions_by_file_name_or_container() {
        let none: &[String] = &[];
        assert!(is_test_function(
            "apps/desktop/src/foo.test.ts",
            "anything",
            none
        ));
        assert!(is_test_function("crates/foo/tests/bar.rs", "helper", none));
        assert!(is_test_function(
            "crates/foo/src/lib.rs",
            "test_thing",
            none
        ));
        assert!(is_test_function("internal/x.go", "TestSomething", none));
        assert!(is_test_function("internal/x.go", "BenchmarkParse", none));
        assert!(is_test_function("internal/x.go", "ExampleUsage", none));
        assert!(!is_test_function("crates/foo/src/lib.rs", "process", none));
        assert!(!is_test_function("apps/desktop/src/foo.ts", "Tester", none));
        assert!(!is_test_function("apps/desktop/src/foo.ts", "tested", none));
        let c = |s: &str| vec![s.to_string()];
        assert!(is_test_function(
            "crates/foo/src/lib.rs",
            "parses",
            &c("tests")
        ));
        assert!(is_test_function(
            "src/Foo.java",
            "validates",
            &c("FooTests")
        ));
        assert!(is_test_function("src/Foo.java", "validates", &c("FooTest")));
        assert!(is_test_function(
            "src/foo/bar.clj",
            "round-trip",
            &c("foo.bar-test")
        ));
        assert!(!is_test_function(
            "crates/foo/src/lib.rs",
            "helper",
            &c("impl_block")
        ));
    }

    fn func(
        name: &str,
        params: u32,
        complexity: f64,
        length: u32,
        start: u32,
        container: &[&str],
    ) -> AnalyzedFunction {
        AnalyzedFunction {
            name: name.into(),
            start_line: start,
            length,
            complexity,
            parameter_count: params,
            nloc: length,
            container_path: container.iter().map(|s| s.to_string()).collect(),
            visibility: "public".into(),
        }
    }

    fn analysis() -> AnalyzeFunctionsResult {
        AnalyzeFunctionsResult {
            sides: vec![
                AnalyzedFileSide {
                    path: "src/foo.ts".into(),
                    side: "base".into(),
                    functions: vec![
                        func("alpha", 1, 3.0, 10, 1, &[]),
                        func("beta", 2, 5.0, 20, 12, &[]),
                        func("gone", 0, 1.0, 4, 33, &[]),
                        func("save", 0, 1.0, 4, 40, &["UserStore"]),
                        func("save", 0, 1.0, 4, 50, &["DocStore"]),
                    ],
                },
                AnalyzedFileSide {
                    path: "src/foo.ts".into(),
                    side: "head".into(),
                    functions: vec![
                        func("alpha", 1, 3.0, 10, 1, &[]),
                        func("beta", 3, 8.0, 22, 12, &[]),
                        func("fresh", 1, 2.0, 70, 28, &[]),
                        func("save", 1, 1.0, 4, 40, &["UserStore"]),
                        func("save", 0, 1.0, 4, 50, &["DocStore"]),
                    ],
                },
            ],
            churn: vec![AnalyzedFileChurn {
                path: "src/foo.ts".into(),
                file_added: 12,
                file_deleted: 4,
                functions: vec![
                    AnalyzedFunctionChurn {
                        name: "beta".into(),
                        container_path: vec![],
                        start_line_head: 12,
                        added_lines: 6,
                        deleted_lines: 2,
                        modified_lines: 2,
                    },
                    AnalyzedFunctionChurn {
                        name: "fresh".into(),
                        container_path: vec![],
                        start_line_head: 28,
                        added_lines: 8,
                        deleted_lines: 0,
                        modified_lines: 0,
                    },
                ],
            }],
            import_deltas: vec![],
        }
    }

    #[test]
    fn functions_are_bucketed_like_the_old_ui_and_carry_churn() {
        let files = vec![ChangedFile {
            path: "src/foo.ts".into(),
            status: "modified".into(),
            additions: 12,
            deletions: 4,
            ..Default::default()
        }];
        let r = build_results(&files, &analysis(), &ZoneRules::from_config(&[]));
        let by = |c: &str, n: &str| r.functions.iter().find(|f| f.container == c && f.name == n);
        assert!(
            by("", "alpha").is_none(),
            "unchanged functions aren't stored"
        );
        let beta = by("", "beta").unwrap();
        assert_eq!(
            (
                beta.status.as_str(),
                beta.signature_changed,
                beta.body_changed
            ),
            ("modified", true, true)
        );
        assert_eq!((beta.params_before, beta.params_after), (Some(2), Some(3)));
        assert_eq!(
            (beta.complexity_delta, beta.length_delta),
            (Some(3.0), Some(2))
        );
        assert_eq!((beta.added_lines, beta.deleted_lines), (Some(6), Some(2)));
        assert!(
            (beta.churn_share.unwrap() - 0.5).abs() < 1e-9,
            "8 of 16 churned lines"
        );
        assert_eq!(by("", "fresh").unwrap().status, "added");
        assert_eq!(by("", "gone").unwrap().status, "deleted");
        let user = by("UserStore", "save").unwrap();
        assert!(user.signature_changed && !user.body_changed);
        assert!(
            by("DocStore", "save").is_none(),
            "same short name in a sibling container doesn't collide"
        );

        let f = &r.files[0];
        assert_eq!(
            (f.status.as_str(), f.additions, f.is_test),
            ("modified", 12, false)
        );
    }

    #[test]
    fn imports_and_zones_are_recorded() {
        use crate::code_analysis::ImportDelta;
        use oxplow_code_deps::{ImportEdge, ImportKind, ZonedImportEdge};
        let zones = ZoneRules::from_config(&[oxplow_config::ZoneRuleConfig {
            patterns: vec!["src/ui/**".into()],
            zone: "ui".into(),
            color: None,
        }]);
        let edge = |module: &str| ZonedImportEdge {
            edge: ImportEdge {
                from_path: "src/ui/a.ts".into(),
                raw: String::new(),
                module: module.into(),
                kind: ImportKind::Import,
                start_line: 3,
                end_line: 3,
            },
            from_zone: "ui".into(),
            to_zone: Some("store".into()),
        };
        let mut a = analysis();
        a.import_deltas = vec![ImportDelta {
            path: "src/ui/a.ts".into(),
            added: vec![edge("../store/x")],
            removed: vec![edge("./old")],
            cross_zone_added: vec![edge("../store/x")],
        }];
        let files = vec![ChangedFile {
            path: "src/ui/a.ts".into(),
            status: "modified".into(),
            ..Default::default()
        }];
        let r = build_results(&files, &a, &zones);
        assert_eq!(r.files[0].zone.as_deref(), Some("ui"));
        let imports: Vec<(&str, &str, bool)> = r
            .imports
            .iter()
            .map(|i| (i.module.as_str(), i.direction.as_str(), i.cross_zone))
            .collect();
        assert_eq!(
            imports,
            vec![("../store/x", "added", true), ("./old", "removed", false)]
        );
    }

    const BEFORE: &str = "fn keep() -> u32 {\n    1\n}\n\nfn grow(a: u32) -> u32 {\n    a\n}\n";
    const AFTER: &str = "fn keep() -> u32 {\n    1\n}\n\nfn grow(a: u32, b: u32) -> u32 {\n    if a > b {\n        a\n    } else {\n        b\n    }\n}\n\nfn fresh() {}\n";

    async fn rows(svc: &crate::Services, sql: &str, id: i64) -> serde_json::Value {
        let out = crate::sql_gateway::SqlGateway::new(svc.db.clone())
            .query_sql(sql, vec![oxplow_db::SqlCell::Int(id)], None)
            .await
            .unwrap();
        serde_json::to_value(out.rows).unwrap()
    }

    #[tokio::test]
    async fn weakened_tests_are_counted_on_both_sides() {
        let f = crate::test_fixtures::services_with_effort().await;
        let root = f.svc.layout.project_dir.clone();
        std::fs::create_dir_all(root.join("tests")).unwrap();
        std::fs::write(
            root.join("tests/it.rs"),
            "#[test]\nfn a() {\n    assert_eq!(1, 1);\n    assert!(true);\n}\n\n#[test]\nfn b() {\n    assert!(true);\n}\n",
        )
        .unwrap();
        crate::test_fixtures::commit_all(&root, "base");
        std::fs::write(
            root.join("tests/it.rs"),
            "#[test]\n#[ignore]\nfn a() {\n    assert_eq!(1, 1);\n}\n",
        )
        .unwrap();
        let sha = crate::test_fixtures::commit_all(&root, "weaken");
        let c = ensure_change(
            &f.svc,
            ChangeTarget::Commit {
                sha,
                stream_id: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            rows(
                &f.svc,
                "SELECT path, tests_before, tests_after, assertions_before, assertions_after, \
                 skips_before, skips_after FROM v_change_test_file WHERE change_id = ?1",
                c.id
            )
            .await,
            serde_json::json!([["tests/it.rs", 2, 1, 3, 1, 0, 1]])
        );
    }

    /// P2.10 (tsk434): a turn diffs its start snapshot → its end snapshot
    /// — everything the turn changed, even across a snapshot taken in the
    /// middle (an effort closing mid-turn).
    #[tokio::test]
    async fn a_turn_diffs_from_where_it_started_to_where_it_ended() {
        let f = crate::test_fixtures::services_with_effort().await;
        let root = f.svc.layout.project_dir.clone();
        let capture = crate::snapshot_capture::SnapshotCaptureService::new(
            f.svc.snapshot_store.clone(),
            f.svc.blobs.clone(),
            root.clone(),
            std::sync::Arc::new(crate::vcs::GitProvider),
            oxplow_domain::StreamId::new(1),
            1_000_000,
            oxplow_fs_watch::WorkspaceFilter::default(),
        )
        .with_settle_duration(std::time::Duration::ZERO)
        .with_predrain_delay(std::time::Duration::ZERO);
        let take = |path: &str, body: &str| {
            let full = root.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(&full, body).unwrap();
            capture.mark_dirty(full, oxplow_fs_watch::WatchEventKind::Other);
            capture.request_snapshot(crate::snapshot_capture::TakeRequest {
                trigger: oxplow_domain::snapshot::SnapshotTrigger::Manual,
                thread_id: None,
                turn_id: None,
                effort_id: None,
                budget: None,
            })
        };
        let start = take("src/a.rs", BEFORE).await.unwrap().unwrap();
        let _mid = take("src/a.rs", AFTER).await.unwrap().unwrap();
        let end = take("src/b.rs", "pub fn b() {}\n").await.unwrap().unwrap();
        let (ended, running): (i64, i64) = f
            .svc
            .db
            .transaction(move |c| {
                let ins = |snap: Option<i64>| {
                    c.execute(
                        "INSERT INTO agent_turn (thread_id, prompt, started_at, start_snapshot_id, snapshot_id)
                         VALUES (1, 'p', '2026-01-01T00:00:00.000000Z', ?1, ?2)",
                        rusqlite::params![start, snap],
                    )
                    .map(|_| c.last_insert_rowid())
                    .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))
                };
                Ok((ins(Some(end))?, ins(None)?))
            })
            .await
            .unwrap();

        let turn_id = oxplow_domain::AgentTurnId::new(ended).to_string();
        let c = ensure_change(
            &f.svc,
            ChangeTarget::Turn {
                turn_id: turn_id.clone(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            (c.kind.as_str(), c.target.as_str(), c.status.as_str()),
            ("turn", ended.to_string().as_str(), "done")
        );
        assert_eq!(
            rows(
                &f.svc,
                "SELECT path, status FROM v_change_file WHERE change_id = ?1 ORDER BY path",
                c.id
            )
            .await,
            serde_json::json!([["src/a.rs", "modified"], ["src/b.rs", "added"]])
        );

        // A turn still running has no end, so no change to analyze.
        let err = ensure_change(
            &f.svc,
            ChangeTarget::Turn {
                turn_id: oxplow_domain::AgentTurnId::new(running).to_string(),
            },
        )
        .await
        .unwrap_err();
        assert!(
            matches!(err, oxplow_domain::DomainError::NotFound),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn a_commit_is_analyzed_once_and_stored() {
        let f = crate::test_fixtures::services_with_effort().await;
        let root = f.svc.layout.project_dir.clone();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/lib.rs"), BEFORE).unwrap();
        crate::test_fixtures::commit_all(&root, "base");
        std::fs::write(root.join("src/lib.rs"), AFTER).unwrap();
        let sha = crate::test_fixtures::commit_all(&root, "change");

        let c = ensure_change(
            &f.svc,
            ChangeTarget::Commit {
                sha: sha.clone(),
                stream_id: None,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            (c.kind.as_str(), c.target.as_str(), c.status.as_str()),
            ("commit", sha.as_str(), "done")
        );
        assert_eq!(
            rows(
                &f.svc,
                "SELECT path, status, additions > 0 FROM v_change_file WHERE change_id = ?1",
                c.id
            )
            .await,
            serde_json::json!([["src/lib.rs", "modified", 1]])
        );
        assert_eq!(
            rows(&f.svc, "SELECT name, status, signature_changed FROM v_change_function WHERE change_id = ?1 ORDER BY name", c.id).await,
            serde_json::json!([["fresh", "added", 0], ["grow", "modified", 1]])
        );
        // HEAD resolves to the same commit, so it's the same change — read,
        // not written: a write announces `v_change`, and a page re-asks on
        // every announcement (tsk1024).
        let mut changes = f.svc.db.subscribe_changes();
        let again = ensure_change(
            &f.svc,
            ChangeTarget::Commit {
                sha: "HEAD".into(),
                stream_id: None,
            },
        )
        .await
        .unwrap();
        assert_eq!((again.id, again.computed_at), (c.id, c.computed_at));
        while let Ok(t) = changes.try_recv() {
            assert!(
                !t.tables.contains("change"),
                "a cached change was written: {:?}",
                t.tables
            );
        }
    }

    #[tokio::test]
    async fn the_working_tree_is_recomputed_when_refreshed() {
        let f = crate::test_fixtures::services_with_effort().await;
        let root = f.svc.layout.project_dir.clone();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/lib.rs"), BEFORE).unwrap();
        crate::test_fixtures::commit_all(&root, "base");
        std::fs::write(root.join("src/lib.rs"), AFTER).unwrap();
        let target = ChangeTarget::Working {
            stream_id: oxplow_domain::StreamId::new(1).to_string(),
        };

        let c = ensure_change(&f.svc, target.clone()).await.unwrap();
        assert_eq!(c.kind, "working");
        assert_eq!(
            rows(
                &f.svc,
                "SELECT path FROM v_change_file WHERE change_id = ?1",
                c.id
            )
            .await,
            serde_json::json!([["src/lib.rs"]])
        );
        std::fs::write(root.join("src/other.rs"), "fn x() {}\n").unwrap();
        let cached = ensure_change(&f.svc, target.clone()).await.unwrap();
        assert_eq!(
            rows(
                &f.svc,
                "SELECT count(*) FROM v_change_file WHERE change_id = ?1",
                cached.id
            )
            .await,
            serde_json::json!([[1]]),
            "ensuring reads the stored analysis; the consumer refreshes it"
        );
        let fresh = refresh_change(&f.svc, target).await.unwrap();
        assert_eq!(fresh.id, c.id);
        assert_eq!(
            rows(
                &f.svc,
                "SELECT path FROM v_change_file WHERE change_id = ?1 ORDER BY path",
                fresh.id
            )
            .await,
            serde_json::json!([["src/lib.rs"], ["src/other.rs"]])
        );
    }

    /// tsk1095: stage one — which files changed, with status and line
    /// counts, and for the working tree git's in-progress operation and
    /// conflicts — is stored on its own, stamped with what it saw, and
    /// leaves the deep analysis alone.
    #[tokio::test]
    async fn refreshing_files_stores_the_file_list_alone() {
        let f = crate::test_fixtures::services_with_effort().await;
        let root = f.svc.layout.project_dir.clone();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/lib.rs"), BEFORE).unwrap();
        crate::test_fixtures::commit_all(&root, "base");
        std::fs::write(root.join("src/lib.rs"), AFTER).unwrap();
        std::fs::write(root.join("src/new.rs"), "fn n() {}\n").unwrap();
        let target = ChangeTarget::Working {
            stream_id: oxplow_domain::StreamId::new(1).to_string(),
        };
        let c = refresh_files(&f.svc, target.clone()).await.unwrap();
        assert_eq!(
            rows(
                &f.svc,
                "SELECT path, status, additions > 0 FROM v_change_file WHERE change_id = ?1 ORDER BY path",
                c.id
            )
            .await,
            serde_json::json!([["src/lib.rs", "modified", 1], ["src/new.rs", "added", 1]])
        );
        assert_eq!(
            rows(
                &f.svc,
                "SELECT files_at IS NOT NULL, files_events_to IS NOT NULL, conflicted, in_progress,
                        (SELECT count(*) FROM v_change_function WHERE change_id = ?1)
                 FROM v_change WHERE id = ?1",
                c.id
            )
            .await,
            serde_json::json!([[1, 1, 0, null, 0]]),
            "stamped; no conflicts; the deep analysis untouched"
        );

        // A merge that stopped on a conflict: the operation and the
        // conflicted file show.
        let git = |args: &[&str]| {
            std::process::Command::new("git")
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .current_dir(&root)
                .output()
                .unwrap()
        };
        git(&["checkout", "-q", "--", "src/lib.rs"]);
        std::fs::remove_file(root.join("src/new.rs")).unwrap();
        git(&["checkout", "-q", "-b", "other"]);
        std::fs::write(root.join("src/lib.rs"), "fn a() -> i32 {\n    3\n}\n").unwrap();
        git(&["commit", "-qam", "other"]);
        git(&["checkout", "-q", "-"]);
        std::fs::write(root.join("src/lib.rs"), "fn a() -> i32 {\n    4\n}\n").unwrap();
        git(&["commit", "-qam", "mine"]);
        git(&["merge", "-q", "other"]);
        let c = refresh_files(&f.svc, target).await.unwrap();
        assert_eq!(
            rows(
                &f.svc,
                "SELECT conflicted, in_progress FROM v_change WHERE id = ?1",
                c.id
            )
            .await,
            serde_json::json!([[1, "merge"]])
        );
    }

    #[tokio::test]
    async fn an_effort_without_a_start_snapshot_is_explained() {
        let f = crate::test_fixtures::services_with_effort().await;
        let err = ensure_change(
            &f.svc,
            ChangeTarget::Effort {
                effort_id: f.effort.to_string(),
            },
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("start snapshot"), "{err}");
    }

    #[tokio::test]
    async fn duplicates_of_changed_files_arrive_after_the_main_analysis() {
        let f = crate::test_fixtures::services_with_effort().await;
        let root = f.svc.layout.project_dir.clone();
        let body = "pub fn tally(items: &[u32]) -> u32 {\n    let mut total = 0;\n    for item in items {\n        if *item > 10 {\n            total += item * 2;\n        } else {\n            total += item + 1;\n        }\n    }\n    total\n}\n";
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), body).unwrap();
        crate::test_fixtures::commit_all(&root, "base");
        std::fs::write(root.join("src/b.rs"), body.replace("tally", "count_up")).unwrap();
        let sha = crate::test_fixtures::commit_all(&root, "copy");
        let c = ensure_change(
            &f.svc,
            ChangeTarget::Commit {
                sha: sha.clone(),
                stream_id: None,
            },
        )
        .await
        .unwrap();
        for _ in 0..200 {
            let got = rows(
                &f.svc,
                "SELECT path, peer_path FROM v_change_duplicate WHERE change_id = ?1",
                c.id,
            )
            .await;
            if got != serde_json::json!([]) {
                assert_eq!(got, serde_json::json!([["src/b.rs", "src/a.rs"]]));
                // The scan is on the code-quality record too, at the commit,
                // with both halves of the pair.
                assert_eq!(
                    rows(
                        &f.svc,
                        "SELECT s.status, s.revision, f.path FROM v_code_quality_scan s \
                         JOIN v_code_quality_finding f ON f.scan_id = s.id WHERE ?1 > 0 ORDER BY f.path",
                        c.id,
                    )
                    .await,
                    serde_json::json!([
                        ["done", format!("git:{sha}"), "src/a.rs"],
                        ["done", format!("git:{sha}"), "src/b.rs"]
                    ])
                );
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("no duplicates stored");
    }

    #[test]
    fn duplicate_scans_run_one_at_a_time_per_change_and_only_the_latest_stores() {
        let q = DupQueue::default();
        let job = |g: i64| DupJob {
            events_to: g,
            root: std::path::PathBuf::from("/r"),
            revision: Revision::Working,
            changed: vec!["a.rs".into()],
        };
        assert!(q.submit(1, job(1)), "first request starts a worker");
        assert!(
            !q.submit(1, job(2)),
            "one already running: queued, not a second worker"
        );
        assert!(
            !q.submit(1, job(3)),
            "a newer request replaces the queued one"
        );
        assert!(q.submit(2, job(1)), "another change runs independently");
        assert_eq!(q.next(1).map(|j| j.events_to), Some(3));
        assert!(q.next(1).is_none(), "drained: the worker stops");
        assert!(q.submit(1, job(4)), "and the next request starts a new one");
    }
}
