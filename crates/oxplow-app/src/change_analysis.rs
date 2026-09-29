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
use crate::endpoint_diff::{compute_diff, endpoint_contents, DiffEndpoint};

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

// "Look here first" weights (formerly interestingness.ts).
const COMPLEXITY_COEFF: f64 = 0.6;
const PARAM_COEFF: f64 = 0.4;
const LONG_NEW_FN_THRESHOLD: i64 = 60;
const LONG_NEW_FN_DIVISOR: f64 = 40.0;
const REASON_THRESHOLD: f64 = 1.2;

fn plural(n: i64, word: &str) -> String {
    if n == 1 {
        format!("{n} {word}")
    } else {
        format!("{n} {word}s")
    }
}

/// The "look here first" score for one file and why it's high: size,
/// complexity spikes, parameter growth and long new functions combine
/// multiplicatively, so one hot factor dominates.
pub fn file_interest(
    additions: i64,
    deletions: i64,
    functions: &[&ChangeFunctionRow],
) -> (f64, Vec<String>) {
    let size = ((1 + additions + deletions) as f64).log2();
    let spiked: Vec<f64> = functions
        .iter()
        .filter(|f| f.body_changed)
        .filter_map(|f| f.complexity_delta)
        .filter(|d| *d > 0.0)
        .collect();
    let complexity_spike: f64 = spiked.iter().sum();
    let grown: Vec<i64> = functions
        .iter()
        .filter(|f| f.signature_changed)
        .filter_map(|f| Some(f.params_after? - f.params_before?))
        .filter(|d| *d > 0)
        .collect();
    let param_spike: i64 = grown.iter().sum();
    let longest_new = functions
        .iter()
        .filter(|f| f.status == "added")
        .filter_map(|f| f.length)
        .filter(|l| *l > LONG_NEW_FN_THRESHOLD)
        .max();
    let long_factor = 1.0
        + longest_new.map_or(0.0, |l| {
            (l - LONG_NEW_FN_THRESHOLD) as f64 / LONG_NEW_FN_DIVISOR
        });
    let complexity_factor = 1.0 + COMPLEXITY_COEFF * complexity_spike;
    let param_factor = 1.0 + PARAM_COEFF * param_spike as f64;
    let score = (1.0 + size) * complexity_factor * param_factor * long_factor;

    let mut reasons = Vec::new();
    if complexity_factor >= REASON_THRESHOLD {
        reasons.push(format!(
            "complexity +{} across {}",
            fmt_num(complexity_spike),
            plural(spiked.len() as i64, "fn")
        ));
    }
    if param_factor >= REASON_THRESHOLD {
        reasons.push(format!(
            "+{} across {}",
            plural(param_spike, "param"),
            plural(grown.len() as i64, "fn")
        ));
    }
    if let Some(l) = longest_new.filter(|_| long_factor >= REASON_THRESHOLD) {
        reasons.push(format!("added {l}-line function"));
    }
    if size >= 5.0 {
        reasons.push(format!("{} lines touched", additions + deletions));
    }
    (score, reasons)
}

fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
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

/// Co-change history cached for (repo, HEAD sha).
type CachedHistory = (
    std::path::PathBuf,
    String,
    std::sync::Arc<oxplow_git::co_change::CoChangeHistory>,
);

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

    let files: Vec<ChangeFileRow> = files
        .iter()
        .map(|f| {
            let fns: Vec<&ChangeFunctionRow> =
                functions.iter().filter(|x| x.path == f.path).collect();
            let (interest, interest_reasons) = file_interest(f.additions, f.deletions, &fns);
            ChangeFileRow {
                path: f.path.clone(),
                status: f.status.clone(),
                additions: f.additions,
                deletions: f.deletions,
                zone: Some(zones.classify(&f.path)),
                is_test: is_test_path(&f.path),
                interest,
                interest_reasons,
            }
        })
        .collect();

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
        co_changes: Vec::new(),
        test_files: Vec::new(),
    }
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
pub fn co_change_rows(
    surprises: Vec<oxplow_git::co_change::FileSurprise>,
) -> Vec<oxplow_db::ChangeCoChangeRow> {
    use oxplow_git::co_change::SurpriseReason;
    surprises
        .into_iter()
        .filter_map(|s| match s.reason {
            SurpriseReason::Normal => None,
            SurpriseReason::UsualCoChangersAbsent { expected } => {
                Some(oxplow_db::ChangeCoChangeRow {
                    path: s.path,
                    reason: "usual-co-changers-absent".into(),
                    expected: Some(expected.join(", ")),
                    dormant_days: None,
                })
            }
            SurpriseReason::Dormant { last_touched_days } => Some(oxplow_db::ChangeCoChangeRow {
                path: s.path,
                reason: "dormant".into(),
                expected: None,
                dormant_days: Some(last_touched_days),
            }),
        })
        .collect()
}

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

/// Tracks which mutable changes (working tree, open efforts) are stale,
/// and which are being computed.
#[derive(Default)]
pub struct ChangeAnalyzer {
    state: std::sync::Mutex<AnalyzerState>,
    /// Duplicate scans: one worker per change at a time.
    dup_queue: DupQueue,
    /// Co-change history for (repo, HEAD): a walk of up to 5000 commits,
    /// so it's reused until HEAD moves.
    history: std::sync::Mutex<Option<CachedHistory>>,
}

/// One duplicate scan to run for a change.
pub(crate) struct DupJob {
    /// The analysis generation it belongs to: its findings are stored only
    /// if that's still the change's latest computation.
    generation: u64,
    root: std::path::PathBuf,
    version: oxplow_tree_source::TreeVersion,
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
    /// Bumped per stream whenever its working tree or refs move.
    stream_gen: std::collections::HashMap<i64, u64>,
    /// The stream generation each mutable change was computed at.
    computed_gen: std::collections::HashMap<i64, u64>,
    running: std::collections::HashSet<i64>,
}

impl ChangeAnalyzer {
    /// The co-change history at `root`'s HEAD, built once per HEAD.
    fn history(
        &self,
        root: &std::path::Path,
    ) -> std::sync::Arc<oxplow_git::co_change::CoChangeHistory> {
        let head = git2::Repository::open(root)
            .ok()
            .and_then(|r| r.head().ok()?.target())
            .map(|o| o.to_string())
            .unwrap_or_default();
        if let Ok(cache) = self.history.lock() {
            if let Some((r, h, hist)) = cache.as_ref() {
                if r == root && *h == head {
                    return hist.clone();
                }
            }
        }
        let hist = std::sync::Arc::new(oxplow_git::co_change::build_history(
            root,
            oxplow_git::co_change::CoChangeOptions::default(),
        ));
        if let Ok(mut cache) = self.history.lock() {
            *cache = Some((root.to_path_buf(), head, hist.clone()));
        }
        hist
    }

    /// The working tree or refs of `stream_id` moved: its mutable changes
    /// are stale.
    pub fn invalidate_stream(&self, stream_id: i64) {
        if let Ok(mut s) = self.state.lock() {
            *s.stream_gen.entry(stream_id).or_default() += 1;
        }
    }
}

/// Quiet period before announcing that a stream's changes went stale.
pub const STALE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(1500);

/// Background: when a stream's working tree or refs move, mark its
/// mutable changes stale and (debounced) emit `ChangeStale`, so pages
/// showing them ask for a fresh analysis.
pub fn spawn_invalidation(state: std::sync::Arc<crate::Services>) {
    use crate::OxplowEvent;
    let mut rx = state.events.subscribe();
    let (stale_tx, mut stale_rx) = tokio::sync::mpsc::unbounded_channel::<i64>();
    let announcer = state.clone();
    tokio::spawn(async move {
        while let Some(first) = stale_rx.recv().await {
            let mut streams = std::collections::BTreeSet::from([first]);
            loop {
                match tokio::time::timeout(STALE_DEBOUNCE, stale_rx.recv()).await {
                    Ok(Some(s)) => {
                        streams.insert(s);
                    }
                    Ok(None) => return,
                    Err(_) => break,
                }
            }
            for stream_id in streams {
                announcer
                    .events
                    .emit(OxplowEvent::ChangeStale { stream_id });
            }
        }
    });
    tokio::spawn(async move {
        loop {
            let stream = match rx.recv().await {
                Ok(OxplowEvent::SnapshotTaken { stream_id: s, .. })
                | Ok(OxplowEvent::GitRefsChanged { stream_id: s }) => s.value(),
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            state.change_analyzer.invalidate_stream(stream);
            let _ = stale_tx.send(stream);
        }
    });
}

/// Analyze `target` if it hasn't been (or, for the working tree and open
/// efforts, if it's stale), store the results, and return the change.
/// While another call is computing the same change this returns it with
/// status `running`; `ChangeAnalyzed` fires when results land.
pub async fn ensure_change(
    svc: &crate::Services,
    target: ChangeTarget,
) -> Result<oxplow_db::ChangeRow, oxplow_domain::DomainError> {
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
    let (stream, kind, key, base, head, mutable) = match target {
        ChangeTarget::Working { stream_id } => {
            let sid = oxplow_domain::StreamId::try_from_str(&stream_id)
                .ok_or_else(|| invalid(format!("not a stream id: {stream_id}")))?;
            let head_label = Some("working tree".to_string());
            (
                sid,
                "working",
                String::new(),
                (
                    Some(DiffEndpoint::Commit { sha: "HEAD".into() }),
                    Some("HEAD".to_string()),
                ),
                (DiffEndpoint::Working, head_label),
                true,
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
            let root = svc.git.resolve_repo_dir(Some(&sid.to_string())).await;
            let repo = git2::Repository::open(&root).map_err(|e| invalid(format!("git: {e}")))?;
            let commit = repo
                .revparse_single(&sha)
                .and_then(|o| o.peel_to_commit())
                .map_err(|e| invalid(format!("no commit `{sha}`: {e}")))?;
            let full = commit.id().to_string();
            let base = commit.parent(0).ok().map(|p| p.id().to_string());
            (
                sid,
                "commit",
                full.clone(),
                (base.clone().map(|sha| DiffEndpoint::Commit { sha }), base),
                (DiffEndpoint::Commit { sha: full.clone() }, Some(full)),
                false,
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
                (
                    Some(DiffEndpoint::Snapshot { snapshot_id: start }),
                    Some(format!("snapshot {start}")),
                ),
                (
                    DiffEndpoint::Snapshot { snapshot_id: end },
                    Some(format!("snapshot {end}")),
                ),
                false,
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
            let open = effort.ended_at.is_none();
            let head = match effort.end_snapshot_id {
                Some(end) if !open => (
                    DiffEndpoint::Snapshot { snapshot_id: end },
                    Some(format!("snapshot {end}")),
                ),
                _ => (DiffEndpoint::Working, Some("working tree".to_string())),
            };
            (
                thread.stream_id,
                "effort",
                eid.value().to_string(),
                (
                    Some(DiffEndpoint::Snapshot { snapshot_id: start }),
                    Some(format!("snapshot {start}")),
                ),
                head,
                open,
            )
        }
    };
    let stream_val = stream.value();
    let (row, head_moved) = svc
        .change_store
        .get_or_create(stream_val, kind, &key, base.1, head.1)
        .await?;
    let generation = {
        let mut st = svc
            .change_analyzer
            .state
            .lock()
            .map_err(|_| invalid("analyzer lock".into()))?;
        if st.running.contains(&row.id) {
            return Ok(row);
        }
        let generation = st.stream_gen.get(&stream_val).copied().unwrap_or(0);
        // Results of another head (an effort analyzed while open, now
        // closed) are stale however they were computed.
        let fresh = row.status == "done"
            && !head_moved
            && (!mutable || st.computed_gen.get(&row.id) == Some(&generation));
        if fresh {
            return Ok(row);
        }
        st.running.insert(row.id);
        generation
    };
    let running = RunningGuard {
        state: &svc.change_analyzer.state,
        id: row.id,
    };
    svc.change_store.set_status(row.id, "running", None).await?;
    let root = svc.git.resolve_repo_dir(Some(&stream.to_string())).await;
    let dup_head = head.0.clone();
    let result = compute(svc, &root, base.0, head.0).await;
    drop(running);
    if result.is_ok() {
        if let Ok(mut st) = svc.change_analyzer.state.lock() {
            st.computed_gen.insert(row.id, generation);
        }
    }
    match result {
        Ok(results) => {
            let changed: Vec<String> = results.files.iter().map(|f| f.path.clone()).collect();
            svc.change_store.store_results(row.id, results).await?;
            svc.events
                .emit(crate::OxplowEvent::ChangeAnalyzed { change_id: row.id });
            spawn_duplicates(svc, row.id, generation, &root, &dup_head, changed);
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

/// Find duplicated blocks between the changed files and anything else in
/// the tree at `head`, in the background (it parses the whole tree), then
/// store them and announce the change again. The scan is also recorded as
/// a code-quality scan with `oxplow.duplicate_lines` facts
/// ([`crate::duplication_scan`]). Snapshot heads (closed efforts) aren't
/// scannable yet, so they get none.
fn spawn_duplicates(
    svc: &crate::Services,
    change_id: i64,
    generation: u64,
    root: &std::path::Path,
    head: &DiffEndpoint,
    changed: Vec<String>,
) {
    use oxplow_tree_source::TreeVersion;
    let version = match head {
        DiffEndpoint::Working => TreeVersion::Disk,
        DiffEndpoint::Commit { sha } => TreeVersion::Ref { r#ref: sha.clone() },
        DiffEndpoint::Snapshot { .. } => return,
    };
    if changed.is_empty() {
        return;
    }
    let job = DupJob {
        generation,
        root: root.to_path_buf(),
        version,
        changed,
    };
    if !svc.change_analyzer.dup_queue.submit(change_id, job) {
        return; // A worker is running; it picks this up next.
    }
    let recorder = crate::duplication_scan::DuplicationRecorder::new(svc);
    let store = svc.change_store.clone();
    let events = svc.events.clone();
    let analyzer = svc.change_analyzer.clone();
    tokio::spawn(async move {
        while let Some(job) = analyzer.dup_queue.next(change_id) {
            let scope = format!("change {change_id}");
            let findings = match recorder
                .record(job.root, job.version, Some(job.changed.clone()), scope)
                .await
            {
                Ok(f) => f,
                Err(error) => {
                    tracing::warn!(change_id, %error, "duplicate scan failed");
                    continue;
                }
            };
            // A newer analysis of this change supersedes these findings.
            let latest = analyzer
                .state
                .lock()
                .map(|st| st.computed_gen.get(&change_id) == Some(&job.generation))
                .unwrap_or(false);
            if !latest {
                continue;
            }
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
            if let Err(error) = store.store_duplicates(change_id, rows).await {
                tracing::warn!(change_id, %error, "storing duplicates failed");
                continue;
            }
            events.emit(crate::OxplowEvent::ChangeAnalyzed { change_id });
        }
    });
}

/// Diff `base` → `head` in `root`, read the changed files' contents, and
/// analyze them.
async fn compute(
    svc: &crate::Services,
    root: &std::path::Path,
    base: Option<DiffEndpoint>,
    head: DiffEndpoint,
) -> Result<ChangeResults, String> {
    let tree = |ep: &Option<DiffEndpoint>| {
        let id = match ep {
            Some(DiffEndpoint::Snapshot { snapshot_id }) => Some(*snapshot_id),
            _ => None,
        };
        async move {
            match id {
                Some(id) => svc
                    .snapshot_store
                    .tree_at(id)
                    .await
                    .map(Some)
                    .map_err(|e| e.to_string()),
                None => Ok(None),
            }
        }
    };
    let mut base_tree = tree(&base).await?;
    let mut head_tree = tree(&Some(head.clone())).await?;
    if let (Some(b), Some(h)) = (base_tree.as_mut(), head_tree.as_mut()) {
        svc.snapshot_store
            .resolve_for_compare(b, h)
            .await
            .map_err(|e| e.to_string())?;
    }
    let (filter, zones) = {
        let cfg = svc.config.read().unwrap_or_else(|e| e.into_inner());
        (
            oxplow_fs_watch::WorkspaceFilter::for_project(
                root,
                &cfg.generated.exclude,
                &cfg.generated.include,
            ),
            ZoneRules::from_config(&cfg.zones),
        )
    };
    let blobs = svc.blobs.clone();
    let root = root.to_path_buf();
    let analyzer = svc.change_analyzer.clone();
    tokio::task::spawn_blocking(move || -> Result<ChangeResults, String> {
        let entries = compute_diff(
            base.clone(),
            head.clone(),
            base_tree.clone(),
            head_tree.clone(),
            &root,
            &blobs,
            &filter,
        )?;
        let analyzed: Vec<String> = entries
            .iter()
            .take(MAX_ANALYZED_FILES)
            .map(|e| e.path.clone())
            .collect();
        let base_contents = match &base {
            Some(b) => endpoint_contents(b, base_tree, &root, &blobs, &filter, analyzed.clone())?,
            None => vec![None; analyzed.len()],
        };
        let head_contents =
            endpoint_contents(&head, head_tree, &root, &blobs, &filter, analyzed.clone())?;
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
                status: e.status,
                additions: e.additions as i64,
                deletions: e.deletions as i64,
                base: None,
                head: None,
            })
            .collect();
        let mut results = build_results(&files, &analysis, &zones);
        results.test_files = test_file_rows(&analyzed, &base_contents, &head_contents, &analysis);
        let paths: Vec<String> = files.iter().map(|f| f.path.clone()).collect();
        let history = analyzer.history(&root);
        results.co_changes = co_change_rows(oxplow_git::co_change::analyze_surprise(
            &history,
            &paths,
            oxplow_git::co_change::DEFAULT_DORMANT_DAYS,
        ));
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
        assert!(f.interest > 4.0, "{f:?}");
        assert!(
            f.interest_reasons
                .iter()
                .any(|x| x.starts_with("complexity +3")),
            "{f:?}"
        );
        assert!(
            f.interest_reasons.iter().any(|x| x.contains("+2 params")),
            "{f:?}"
        );
        assert!(
            f.interest_reasons
                .iter()
                .any(|x| x == "added 70-line function"),
            "{f:?}"
        );
    }

    #[test]
    fn a_routine_file_scores_low_with_no_reasons() {
        let (score, reasons) = file_interest(3, 1, &[]);
        assert!(score < 4.0, "{score}");
        assert!(reasons.is_empty());
        let (_, reasons) = file_interest(40, 20, &[]);
        assert_eq!(reasons, vec!["60 lines touched"]);
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
        let out = oxplow_db::SemanticLayer::new(svc.db.clone())
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
        // HEAD resolves to the same commit, so it's the same change.
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
    }

    #[tokio::test]
    async fn the_working_tree_is_recomputed_only_when_stale() {
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
            "not stale yet, so not recomputed"
        );
        f.svc.change_analyzer.invalidate_stream(1);
        let fresh = ensure_change(&f.svc, target).await.unwrap();
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

    #[test]
    fn only_surprising_files_become_co_change_rows() {
        use oxplow_git::co_change::{FileSurprise, SurpriseReason};
        let rows = co_change_rows(vec![
            FileSurprise {
                path: "a.rs".into(),
                reason: SurpriseReason::Normal,
            },
            FileSurprise {
                path: "b.rs".into(),
                reason: SurpriseReason::UsualCoChangersAbsent {
                    expected: vec!["c.rs".into(), "d.rs".into()],
                },
            },
            FileSurprise {
                path: "e.rs".into(),
                reason: SurpriseReason::Dormant {
                    last_touched_days: 120,
                },
            },
        ]);
        assert_eq!(
            rows,
            vec![
                oxplow_db::ChangeCoChangeRow {
                    path: "b.rs".into(),
                    reason: "usual-co-changers-absent".into(),
                    expected: Some("c.rs, d.rs".into()),
                    dormant_days: None,
                },
                oxplow_db::ChangeCoChangeRow {
                    path: "e.rs".into(),
                    reason: "dormant".into(),
                    expected: None,
                    dormant_days: Some(120),
                },
            ]
        );
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
        let mut rx = f.svc.events.subscribe();
        let c = ensure_change(
            &f.svc,
            ChangeTarget::Commit {
                sha,
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
                let mut announced = 0;
                while let Ok(ev) = rx.try_recv() {
                    if matches!(ev, crate::OxplowEvent::ChangeAnalyzed { change_id } if change_id == c.id)
                    {
                        announced += 1;
                    }
                }
                assert_eq!(
                    announced, 2,
                    "once for the analysis, once for its duplicates"
                );
                // The scan is on the code-quality record too, at the commit,
                // with both halves of the pair.
                assert_eq!(
                    rows(
                        &f.svc,
                        "SELECT s.status, s.tree_version_kind, f.path FROM v_code_quality_scan s \
                         JOIN v_code_quality_finding f ON f.scan_id = s.id WHERE ?1 > 0 ORDER BY f.path",
                        c.id,
                    )
                    .await,
                    serde_json::json!([["done", "ref", "src/a.rs"], ["done", "ref", "src/b.rs"]])
                );
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("no duplicates stored");
    }

    #[tokio::test]
    async fn working_tree_events_mark_changes_stale_and_announce_it() {
        let f = crate::test_fixtures::services_with_effort().await;
        let mut rx = f.svc.events.subscribe();
        spawn_invalidation(f.svc.clone());
        tokio::task::yield_now().await;
        for _ in 0..3 {
            f.svc.events.emit(crate::OxplowEvent::GitRefsChanged {
                stream_id: oxplow_domain::StreamId::new(1),
            });
        }
        let got = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            let mut stale = 0;
            loop {
                match rx.recv().await {
                    Ok(crate::OxplowEvent::ChangeStale { stream_id: 1 }) => {
                        stale += 1;
                        // Debounced: a burst yields one announcement.
                        tokio::time::sleep(STALE_DEBOUNCE * 2).await;
                        while let Ok(ev) = rx.try_recv() {
                            if matches!(ev, crate::OxplowEvent::ChangeStale { .. }) {
                                stale += 1;
                            }
                        }
                        return stale;
                    }
                    Ok(_) => {}
                    Err(_) => return 0,
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(got, 1);
        let generation = f
            .svc
            .change_analyzer
            .state
            .lock()
            .unwrap()
            .stream_gen
            .get(&1)
            .copied();
        assert!(generation.unwrap_or(0) >= 1);
    }

    #[test]
    fn duplicate_scans_run_one_at_a_time_per_change_and_only_the_latest_stores() {
        let q = DupQueue::default();
        let job = |g: u64| DupJob {
            generation: g,
            root: std::path::PathBuf::from("/r"),
            version: oxplow_tree_source::TreeVersion::Disk,
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
        assert_eq!(q.next(1).map(|j| j.generation), Some(3));
        assert!(q.next(1).is_none(), "drained: the worker stops");
        assert!(q.submit(1, job(4)), "and the next request starts a new one");
    }
}
