//! The `change.analyze` pump consumer (P7.B4): keeps the mutable changes
//! — a stream's working tree, its open efforts and its ranges headed at
//! the working tree — analyzed as the stream moves.
//!
//! On a `snapshot.taken` that recorded files (not `unchanged`) or a
//! `vcs.head.moved`, it first lists the changed files of the event's
//! stream's `working` change and each open effort's change on it (stage
//! one, [`change_analysis::refresh_files`](crate::change_analysis::refresh_files):
//! cheap, so the lists are current before anything slow runs), then
//! recomputes their deep analysis
//! ([`change_analysis::refresh_change`](crate::change_analysis::refresh_change)).
//! A burst analyzes once: an event is skipped when a newer one that would
//! trigger it for the same stream is already logged — that one recomputes.
//! A turn's end take that recorded files runs the deep analysis at once
//! (the turn's end itself doesn't: its take says whether anything changed),
//! and an analysis whose inputs haven't moved since it last ran keeps what's
//! stored (`change.analyzed_from`).
//! On `effort.finished` it recomputes that effort's change, now against its
//! end snapshot: the effort closed before its end take, so no take event
//! reached it while it was open (tsk710; what `effort_churn`, `after:
//! [change.analyze]`, reads). An effort with no start snapshot (opened and
//! closed at once, after the fact) has no change, so its finish is done.
//! The results land in `v_change*` (`DatasetsChanged` announces them); an
//! analysis already running for a change defers the event (`Busy`), and a
//! failure is a dead letter naming the stream.
//!
//! It holds `Services` weakly: the pump that runs it is part of it.

use std::sync::{Arc, Weak};
use std::time::Duration;

use async_trait::async_trait;
use oxplow_domain::{DomainError, StoredEvent};

use crate::change_analysis::ChangeTarget;
use crate::event_pump::AsyncEventConsumer;
use crate::Services;

/// The consumer's name: its checkpoint and dead letters.
pub const NAME: &str = "change.analyze";

/// The changes of `stream` that move with its working tree: its `working`
/// change, then each `range` change whose head is the working tree.
async fn working_targets(
    svc: &Services,
    stream: oxplow_domain::StreamId,
) -> Result<Vec<ChangeTarget>, DomainError> {
    let ranges = svc.change_store.list_working_ranges(stream.value()).await?;
    Ok(std::iter::once(ChangeTarget::Working {
        stream_id: stream.to_string(),
    })
    .chain(
        ranges
            .iter()
            .filter_map(crate::change_analysis::range_target),
    )
    .collect())
}

const SNAPSHOT_TAKEN: &str = "snapshot.taken";
const HEAD_MOVED: &str = "vcs.head.moved";
const EFFORT_FINISHED: &str = "effort.finished";
/// When a mutable change's deep analysis runs: its file list is
/// listed on every move (stage one); the deep analysis once a burst of
/// takes settles (20 s), at most every 2 minutes, and at once when HEAD
/// moves or an agent's turn ends with changes (its end take recorded
/// files: [`forces`]). A finishing effort's runs at once.
pub fn deep_pacing() -> oxplow_config::collectors::Pacing {
    oxplow_config::collectors::Pacing {
        settle_secs: Some(20),
        at_most_secs: Some(120),
        idle_secs: None,
        force: vec![HEAD_MOVED.into()],
    }
}

/// Whether `event` runs the deep analysis at once rather than paced: HEAD
/// moving, or a turn's end take — the moment a person looks at what the
/// turn did. The turn's end itself doesn't: whether anything changed is
/// its take's to say.
fn forces(event: &StoredEvent) -> bool {
    !deep_pacing().defers(&event.envelope.event_type)
        || (event.envelope.event_type == SNAPSHOT_TAKEN
            && event.envelope.payload["trigger"].as_str() == Some("turn_end"))
}

/// The paced job of `target`'s deep analysis in `pending_run`: owner
/// `core`, id `change/working/<stream>`, `change/effort/<effort>` or
/// `change/range/<stream>/<base>..<head>` (a range of a known stream,
/// its sides as its stored key spells them).
pub fn job_id(target: &ChangeTarget) -> Option<String> {
    match target {
        ChangeTarget::Working { stream_id } => Some(format!("change/working/{stream_id}")),
        ChangeTarget::Effort { effort_id } => Some(format!("change/effort/{effort_id}")),
        ChangeTarget::Range {
            stream_id: Some(stream_id),
            base,
            head,
        } => Some(format!(
            "change/range/{stream_id}/{}..{head}",
            base.as_deref().unwrap_or_default()
        )),
        _ => None,
    }
}

/// The target a `change/…` job id names.
pub fn target_of(job: &str) -> Option<ChangeTarget> {
    let rest = job.strip_prefix("change/")?;
    let (kind, id) = rest.split_once('/')?;
    match kind {
        "working" => Some(ChangeTarget::Working {
            stream_id: id.to_string(),
        }),
        "effort" => Some(ChangeTarget::Effort {
            effort_id: id.to_string(),
        }),
        "range" => {
            let (stream_id, key) = id.split_once('/')?;
            let (base, head) = key.split_once("..")?;
            Some(ChangeTarget::Range {
                stream_id: Some(stream_id.to_string()),
                base: (!base.is_empty()).then(|| base.to_string()),
                head: head.to_string(),
            })
        }
        _ => None,
    }
}

/// Keep every stream's working-tree file lists in `v_change_file` current
/// — its `working` change's and its ranges' headed at the working tree —
/// where no snapshot says so: once at boot (so a stream has its `working`
/// change before anything is edited), and whenever its refs move or a VCS
/// write (stage, discard, commit, …) changes its index. Stage one only —
/// the deep analysis stays the pump consumer's. Runs until the services go.
pub async fn watch_working_files(svc: Arc<Services>) {
    follow_working_files(svc, RECONCILE_AFTER).await;
}

/// How long a stream stays quiet after a relist of some paths before its whole
/// list is made again. A partial list can't see what no watch reports: `git add`
/// from a terminal changes only `.git/index`.
const RECONCILE_AFTER: Duration = Duration::from_secs(60);

async fn follow_working_files(svc: Arc<Services>, reconcile_after: Duration) {
    use crate::ref_moves::{Moved, WorkingMove};
    use oxplow_domain::stores::StreamStore as _;
    let streams = svc.stream_store.list().await.unwrap_or_default();
    // Listen before the first pass, so a move during it isn't lost.
    let mut refs = svc.ref_moves.subscribe();
    let mut index = svc.ref_moves.subscribe_index();
    let targets = |stream: oxplow_domain::StreamId| {
        let svc = svc.clone();
        async move {
            working_targets(&svc, stream).await.unwrap_or_else(|error| {
                tracing::debug!(%error, %stream, "listing a stream's ranges failed");
                Vec::new()
            })
        }
    };
    let list = |stream: oxplow_domain::StreamId| {
        let (svc, targets) = (svc.clone(), &targets);
        async move {
            for target in targets(stream).await {
                if let Err(error) = crate::change_analysis::refresh_files(&svc, target).await {
                    tracing::debug!(%error, %stream, "listing a working tree's files failed");
                }
            }
        }
    };
    // Its paths alone where that can be trusted; each target says.
    let list_paths = |stream: oxplow_domain::StreamId,
                      paths: std::collections::BTreeSet<String>| {
        let (svc, targets) = (svc.clone(), &targets);
        async move {
            for target in targets(stream).await {
                let partial =
                    crate::change_analysis::refresh_paths(&svc, target.clone(), &paths).await;
                if !matches!(partial, Ok(true)) {
                    if let Err(error) = crate::change_analysis::refresh_files(&svc, target).await {
                        tracing::debug!(%error, %stream, "listing a working tree's files failed");
                    }
                }
            }
        }
    };
    for s in &streams {
        if s.archived_at.is_none() {
            list(s.id).await;
        }
    }
    // The streams listed by paths since their whole list was last made, and
    // when each last moved.
    let mut unreconciled: std::collections::HashMap<oxplow_domain::StreamId, tokio::time::Instant> =
        std::collections::HashMap::new();
    loop {
        let due = unreconciled.values().min().map(|at| *at + reconcile_after);
        let moved = tokio::select! {
            m = refs.recv() => m,
            m = index.recv() => m,
            () = tokio::time::sleep_until(due.unwrap_or_else(tokio::time::Instant::now)), if due.is_some() => {
                let now = tokio::time::Instant::now();
                let ready: Vec<_> = unreconciled
                    .iter()
                    .filter(|(_, at)| **at + reconcile_after <= now)
                    .map(|(stream, _)| *stream)
                    .collect();
                for stream in ready {
                    unreconciled.remove(&stream);
                    list(stream).await;
                }
                continue;
            }
        };
        let Some(first) = moved else { break };
        // A burst of edits queues many moves: list each stream once.
        let mut heard = vec![first];
        heard.extend(refs.pending());
        heard.extend(index.pending());
        if heard.contains(&Moved::Missed) {
            for s in svc.stream_store.list().await.unwrap_or_default() {
                if s.archived_at.is_none() {
                    let _ = svc.ref_moves.take_working(s.id);
                    unreconciled.remove(&s.id);
                    list(s.id).await;
                }
            }
            continue;
        }
        let mut streams: Vec<_> = heard
            .into_iter()
            .filter_map(|m| match m {
                Moved::Stream(s) => Some(s),
                Moved::Missed => None,
            })
            .collect();
        streams.sort_by_key(|s| s.value());
        streams.dedup();
        for s in streams {
            match svc.ref_moves.take_working(s) {
                WorkingMove::Full => {
                    unreconciled.remove(&s);
                    list(s).await;
                }
                WorkingMove::Paths(paths) => {
                    list_paths(s, paths).await;
                    unreconciled.insert(s, tokio::time::Instant::now());
                }
            }
        }
    }
}

/// The owner of core's paced jobs.
pub const CORE: &str = "core";

pub struct ChangeReactor {
    services: Weak<Services>,
}

impl ChangeReactor {
    pub fn new(services: Weak<Services>) -> Self {
        Self { services }
    }
}

/// Register the consumer on `svc`'s pump (boot, before it spawns).
pub fn register(svc: &Arc<Services>) {
    svc.event_pump
        .register_async(Arc::new(ChangeReactor::new(Arc::downgrade(svc))));
}

/// Whether `event` moves its stream's mutable changes: a take that
/// recorded files, or HEAD moving.
fn moves_the_stream(event: &StoredEvent) -> bool {
    match event.envelope.event_type.as_str() {
        SNAPSHOT_TAKEN => {
            let p = &event.envelope.payload;
            !p["unchanged"].as_bool().unwrap_or(false) && p["file_count"].as_u64().unwrap_or(0) > 0
        }
        HEAD_MOVED => true,
        _ => false,
    }
}

/// Whether a newer event that moves `stream` is already logged after
/// `seq`: it recomputes, so this one needn't.
async fn superseded(svc: &Services, stream: i64, seq: i64) -> Result<bool, DomainError> {
    svc.db
        .read(move |c| {
            c.query_row(
                "SELECT EXISTS (SELECT 1 FROM event_log
                   WHERE seq > ?1 AND stream_id = ?2 AND payload_expired_at IS NULL
                     AND (type = 'vcs.head.moved'
                          OR (type = 'snapshot.taken'
                              AND json_extract(payload, '$.unchanged') = 0
                              AND json_extract(payload, '$.file_count') > 0)))",
                rusqlite::params![seq, stream],
                |r| r.get::<_, bool>(0),
            )
            .map_err(oxplow_db::map_sql_err)
        })
        .await
}

#[async_trait]
impl AsyncEventConsumer for ChangeReactor {
    fn name(&self) -> &'static str {
        NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        matches!(event_type, SNAPSHOT_TAKEN | HEAD_MOVED | EFFORT_FINISHED)
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        let Some(svc) = self.services.upgrade() else {
            return Err(DomainError::Busy("services are shutting down".into()));
        };
        if event.envelope.event_type == EFFORT_FINISHED {
            let effort = crate::effort_lifecycle::effort_of(event)?;
            // One with no start snapshot (opened and closed at once, after
            // the fact) has no change to analyze: done, not a dead letter.
            let started = oxplow_db::EffortStore::get_effort(&*svc.effort_store, &effort).await?;
            if started.is_none_or(|e| e.start_snapshot_id.is_none()) {
                return Ok(());
            }
            return deep_now(
                &svc,
                ChangeTarget::Effort {
                    effort_id: effort.to_string(),
                },
                event.seq,
            )
            .await
            .map_err(|e| match e {
                DomainError::Busy(_) => e,
                e => DomainError::Invalid(format!("analyzing finished {effort}: {e}")),
            });
        }
        let Some(stream) = event.envelope.anchors.stream_id else {
            return Ok(());
        };
        if !moves_the_stream(event) || superseded(&svc, stream.value(), event.seq).await? {
            return Ok(());
        }
        let failed = |e: DomainError| match e {
            DomainError::Busy(_) => e,
            e => DomainError::Invalid(format!("analyzing stream {stream}: {e}")),
        };
        // Stage one first, for every mutable change: the file lists are
        // current before any deep analysis begins. A thread's take moves
        // only its own effort's files; a take no thread made (the person's
        // edits, a head move) may move any open effort's.
        let thread = event.envelope.anchors.thread_id;
        let open: Vec<_> = svc
            .effort_store
            .list_open_for_stream(stream)
            .await?
            .into_iter()
            .filter(|e| thread.is_none_or(|t| e.thread_id == t))
            // An effort with no start snapshot has nothing to diff.
            .filter(|e| e.start_snapshot_id.is_some())
            .collect();
        let mut targets = working_targets(&svc, stream).await?;
        targets.extend(open.into_iter().map(|e| ChangeTarget::Effort {
            effort_id: e.id.to_string(),
        }));
        for target in &targets {
            crate::change_analysis::refresh_files(&svc, target.clone())
                .await
                .map_err(failed)?;
        }
        // The deep analysis: at once when this event forces it, else
        // paced (`crate::pacing` runs it once due).
        let forced = forces(event);
        let now = oxplow_domain::Timestamp::now().to_text();
        for target in targets {
            if forced {
                deep_now(&svc, target, event.seq).await.map_err(failed)?;
            } else if let Some(job) = job_id(&target) {
                svc.collector_store
                    .mark_pending(CORE, &job, event.seq, &now)
                    .await?;
            }
        }
        Ok(())
    }
}

/// Analyze `target` now and clear its paced job: this run covers it.
async fn deep_now(svc: &Services, target: ChangeTarget, seq: i64) -> Result<(), DomainError> {
    let job = job_id(&target);
    crate::change_analysis::refresh_change(svc, target).await?;
    if let Some(job) = job {
        svc.collector_store.clear_pending(CORE, &job, seq).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::events::schema::{
        SnapshotTaken, SnapshotTakenV3, VcsHeadMoved, VcsHeadMovedV2,
    };
    use oxplow_domain::events::Anchors;
    use oxplow_domain::{Envelope, StreamId};

    const BEFORE: &str = "fn a() -> i32 {\n    1\n}\n";

    async fn log(svc: &Services, env: Envelope) -> StoredEvent {
        let env = env.with_anchors(Anchors {
            stream_id: Some(StreamId::new(1)),
            ..Anchors::default()
        });
        let id = env.id.clone();
        svc.event_log_store.append(env).await.unwrap();
        svc.event_log_store.get(id).await.unwrap().unwrap()
    }

    fn taken(unchanged: bool, file_count: u32) -> Envelope {
        taken_by(
            oxplow_domain::snapshot::SnapshotTrigger::Quiet,
            unchanged,
            file_count,
        )
    }

    fn taken_by(
        trigger: oxplow_domain::snapshot::SnapshotTrigger,
        unchanged: bool,
        file_count: u32,
    ) -> Envelope {
        Envelope::typed::<SnapshotTaken>(
            "system",
            &SnapshotTakenV3 {
                stream: "str1".into(),
                snapshot: 1,
                parent: None,
                trigger,
                unchanged,
                file_count,
                elapsed_ms: 1,
                budget_ms: None,
                over_budget: false,
            },
        )
    }

    fn head_moved() -> Envelope {
        Envelope::typed::<VcsHeadMoved>(
            "system",
            &VcsHeadMovedV2 {
                stream: "str1".into(),
                snapshot: 1,
                from: None,
                to: "abc".into(),
            },
        )
    }

    async fn working_paths(svc: &Services) -> serde_json::Value {
        let out = svc
            .sql
            .query_sql(
                "SELECT f.path FROM v_change c JOIN v_change_file f ON f.change_id = c.id
                 WHERE c.kind = 'working' ORDER BY f.path",
                vec![],
                None,
            )
            .await
            .unwrap();
        serde_json::to_value(out.rows).unwrap()
    }

    /// A repo with one committed file and an uncommitted edit.
    fn repo(root: &std::path::Path) {
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/lib.rs"), BEFORE).unwrap();
        crate::test_fixtures::commit_all(root, "base");
        std::fs::write(root.join("src/lib.rs"), "fn a() -> i32 {\n    2\n}\n").unwrap();
    }

    /// A take one thread's turn made refreshes that thread's effort, not
    /// every open effort on the stream; a take no thread made (the
    /// person's edits) refreshes them all.
    #[tokio::test]
    async fn a_threads_take_refreshes_only_its_effort() {
        use oxplow_db::EffortStore as _;
        let f = crate::thread_checkpoint::tests::with_baseline().await;
        let stream = StreamId::new(1);
        let start = f
            .svc
            .snapshot_store
            .latest_snapshot_id_for_stream(stream)
            .await
            .unwrap()
            .unwrap();
        f.svc
            .effort_store
            .set_start_snapshot(&f.effort, start)
            .await
            .unwrap();
        f.svc
            .db
            .transaction(|c| {
                c.execute_batch(
                    "INSERT INTO threads (id, stream_id, title, created_at, updated_at)
                       VALUES (7, 1, 'other',
                               '2026-01-01T00:00:00.000000Z', '2026-01-01T00:00:00.000000Z');",
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        let other = f
            .svc
            .effort_store
            .start(
                "oxplow:oxplow/work_item?id=B-1&provider=issues",
                &oxplow_domain::ThreadId::new(7),
                Some(start),
            )
            .await
            .unwrap();
        let reactor = ChangeReactor::new(Arc::downgrade(&f.svc));
        let efforts_analyzed = || async {
            let out = f
                .svc
                .sql
                .query_sql(
                    "SELECT target FROM v_change WHERE kind = 'effort' ORDER BY target",
                    vec![],
                    None,
                )
                .await
                .unwrap();
            serde_json::to_value(out.rows).unwrap()
        };
        let mut mine = log(&f.svc, taken(false, 1)).await;
        mine.envelope.anchors.thread_id = Some(f.thread);
        reactor.handle(&mine).await.unwrap();
        assert_eq!(
            efforts_analyzed().await,
            serde_json::json!([[f.effort.value().to_string()]])
        );
        reactor
            .handle(&log(&f.svc, taken(false, 1)).await)
            .await
            .unwrap();
        assert_eq!(
            efforts_analyzed().await,
            serde_json::json!([
                [f.effort.value().to_string()],
                [other.id.value().to_string()]
            ])
        );
    }

    /// A move lists the working tree's files first, on their own:
    /// even while a deep analysis of it is still running (the deep step
    /// defers, `Busy`), the file list is current.
    #[tokio::test]
    async fn a_move_lists_the_files_even_while_the_analysis_runs() {
        let f = crate::test_fixtures::services_with_effort().await;
        repo(&f.svc.layout.project_dir);
        let reactor = ChangeReactor::new(Arc::downgrade(&f.svc));
        let ev = log(&f.svc, taken(false, 1)).await;
        reactor.handle(&ev).await.unwrap();
        let id = f
            .svc
            .sql
            .query_sql(
                "SELECT id FROM v_change WHERE kind = 'working'",
                vec![],
                None,
            )
            .await
            .unwrap();
        let id = match id.rows[0][0] {
            oxplow_db::SqlCell::Int(n) => n,
            ref other => panic!("{other:?}"),
        };
        crate::change_analysis::hold_running_for_tests(&f.svc, id);
        std::fs::write(f.svc.layout.project_dir.join("src/other.rs"), "fn x() {}\n").unwrap();
        let ev = log(&f.svc, taken(false, 1)).await;
        reactor.handle(&ev).await.unwrap();
        assert_eq!(
            working_paths(&f.svc).await,
            serde_json::json!([["src/lib.rs"], ["src/other.rs"]])
        );
    }

    /// The working tree's files say which are staged and which untracked,
    /// and a deep analysis of them keeps that; an effort's say neither.
    #[tokio::test]
    async fn the_working_files_say_which_are_staged_and_which_untracked() {
        let f = crate::test_fixtures::services_with_effort().await;
        let root = f.svc.layout.project_dir.clone();
        repo(&root);
        std::fs::write(root.join("src/staged.rs"), "fn s() {}\n").unwrap();
        std::fs::write(root.join("src/loose.rs"), "fn l() {}\n").unwrap();
        let out = std::process::Command::new("git")
            .args(["add", "src/staged.rs"])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(out.status.success());
        let reactor = ChangeReactor::new(Arc::downgrade(&f.svc));
        let ev = log(&f.svc, taken(false, 1)).await;
        reactor.handle(&ev).await.unwrap();
        let flags = || async {
            let out = f
                .svc
                .sql
                .query_sql(
                    "SELECT f.path, f.staged, f.untracked FROM v_change c
                     JOIN v_change_file f ON f.change_id = c.id
                     WHERE c.kind = 'working' ORDER BY f.path",
                    vec![],
                    None,
                )
                .await
                .unwrap();
            serde_json::to_value(out.rows).unwrap()
        };
        let want = serde_json::json!([
            ["src/lib.rs", 0, 0],
            ["src/loose.rs", 0, 1],
            ["src/staged.rs", 1, 0]
        ]);
        assert_eq!(flags().await, want);
        // The deep analysis rewrites the rows and keeps the flags.
        crate::change_analysis::refresh_change(
            &f.svc,
            ChangeTarget::Working {
                stream_id: f.svc.streams.list_streams().await.unwrap()[0]
                    .id
                    .to_string(),
            },
        )
        .await
        .unwrap();
        assert_eq!(flags().await, want);
    }

    /// What the list says of every path, for comparing two ways of making it.
    async fn working_rows(svc: &Services) -> Vec<Vec<oxplow_db::SqlCell>> {
        svc.sql
            .query_sql(
                "SELECT f.path, f.status, f.additions, f.deletions, f.staged, f.untracked
                 FROM v_change c JOIN v_change_file f ON f.change_id = c.id
                 WHERE c.kind = 'working' ORDER BY f.path",
                Vec::new(),
                None,
            )
            .await
            .unwrap()
            .rows
    }

    /// Looking only at the paths that changed leaves the list as making the
    /// whole of it would, through every kind of change.
    #[tokio::test]
    async fn a_relist_of_some_paths_gives_what_a_full_relist_gives() {
        use crate::change_analysis::{refresh_files, refresh_paths, ChangeTarget};
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        repo(&root);
        let write = |rel: &str, text: &str| {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(".gitignore", "ignored.txt\n");
        for path in [
            "keep.txt",
            "staged.txt",
            "gone.txt",
            "moved.txt",
            "dir/one.txt",
            "dir/two.txt",
        ] {
            write(path, "one\ntwo\n");
        }
        crate::test_fixtures::commit_all(&root, "files");
        let stream = svc.streams.list_streams().await.unwrap()[0].id.to_string();
        let target = || ChangeTarget::Working {
            stream_id: stream.clone(),
        };
        refresh_files(svc, target()).await.unwrap();
        let index = |add: &[&str], remove: &[&str]| {
            let repo = git2::Repository::open(&root).unwrap();
            let mut index = repo.index().unwrap();
            for path in add {
                index.add_path(std::path::Path::new(path)).unwrap();
            }
            for path in remove {
                index.remove_path(std::path::Path::new(path)).unwrap();
            }
            index.write().unwrap();
        };
        // Each step changes the tree, names the paths the watch would hear, and
        // must leave what a whole relist would.
        let check = {
            let target = &target;
            move |step: &'static str, paths: &[&str]| {
                let paths: std::collections::BTreeSet<String> =
                    paths.iter().map(|p| p.to_string()).collect();
                async move {
                    assert!(
                        refresh_paths(svc, target(), &paths).await.unwrap(),
                        "{step}: it could look at paths alone"
                    );
                    let partial = working_rows(svc).await;
                    refresh_files(svc, target()).await.unwrap();
                    assert_eq!(partial, working_rows(svc).await, "{step}");
                    partial
                }
            }
        };

        write("keep.txt", "one\nchanged\n");
        assert!(!check("an edit", &["keep.txt"]).await.is_empty());
        write("new/deep/new.txt", "new\n");
        check("a new file", &["new/deep/new.txt"]).await;
        std::fs::remove_file(root.join("gone.txt")).unwrap();
        check("a deletion", &["gone.txt"]).await;
        write("keep.txt", "one\ntwo\n");
        check("an edit undone: its row goes", &["keep.txt"]).await;
        write("staged.txt", "staged\n");
        index(&["staged.txt"], &[]);
        check("staged", &["staged.txt"]).await;
        write("staged.txt", "staged, then changed\n");
        check("staged, then changed again", &["staged.txt"]).await;
        write("ignored.txt", "x\n");
        check("an ignored file", &["ignored.txt"]).await;
        std::fs::rename(root.join("moved.txt"), root.join("renamed.txt")).unwrap();
        check("a rename", &["moved.txt", "renamed.txt"]).await;
        write("staged-moved.txt", "one\ntwo\nthree\nfour\n");
        index(&["staged-moved.txt"], &[]);
        crate::test_fixtures::commit_index(&root, "one to rename");
        refresh_files(svc, target()).await.unwrap();
        std::fs::rename(
            root.join("staged-moved.txt"),
            root.join("staged-renamed.txt"),
        )
        .unwrap();
        index(&["staged-renamed.txt"], &["staged-moved.txt"]);
        check(
            "a staged rename",
            &["staged-moved.txt", "staged-renamed.txt"],
        )
        .await;
        // A folder moved is one event for the folder, none for what is in it.
        std::fs::rename(root.join("dir"), root.join("elsewhere")).unwrap();
        check("a folder moved", &["dir", "elsewhere"]).await;
        std::fs::rename(root.join("elsewhere"), root.join("dir")).unwrap();
        check("a folder moved back", &["dir", "elsewhere"]).await;
        svc.config.write().unwrap().generated.exclude = vec!["new".into()];
        check("what generated excludes", &["new/deep/new.txt"]).await;
        // Lifting it, a rescan makes the whole list again.
        svc.config.write().unwrap().generated = Default::default();
        refresh_files(svc, target()).await.unwrap();
        write("ignored.txt", "y\n");
        svc.config.write().unwrap().generated.include = vec!["ignored.txt".into()];
        check("what generated includes", &["ignored.txt"]).await;
    }

    /// The whole list is made when a partial one couldn't be trusted.
    #[tokio::test]
    async fn a_merge_in_progress_or_a_moved_head_takes_the_full_relist() {
        use crate::change_analysis::{refresh_files, refresh_paths, ChangeTarget};
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        repo(&root);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        crate::test_fixtures::commit_all(&root, "a");
        let stream = svc.streams.list_streams().await.unwrap()[0].id.to_string();
        let target = || ChangeTarget::Working {
            stream_id: stream.clone(),
        };
        let paths: std::collections::BTreeSet<String> = ["a.txt".to_string()].into();
        assert!(
            !refresh_paths(svc, target(), &paths).await.unwrap(),
            "no list yet"
        );
        refresh_files(svc, target()).await.unwrap();
        assert!(refresh_paths(svc, target(), &paths).await.unwrap());

        std::fs::write(root.join("a.txt"), "b\n").unwrap();
        crate::test_fixtures::commit_all(&root, "b");
        assert!(
            !refresh_paths(svc, target(), &paths).await.unwrap(),
            "HEAD moved"
        );
        refresh_files(svc, target()).await.unwrap();

        let git_dir = git2::Repository::open(&root).unwrap().path().to_path_buf();
        std::fs::write(git_dir.join("MERGE_HEAD"), "0\n").unwrap();
        assert!(
            !refresh_paths(svc, target(), &paths).await.unwrap(),
            "a merge under way"
        );
    }

    /// A range headed at the working tree follows by paths too, and takes the
    /// whole list for a folder, whose insides it has no status to name.
    #[tokio::test]
    async fn a_range_headed_at_the_working_tree_follows_by_paths() {
        use crate::change_analysis::{refresh_files, refresh_paths, ChangeTarget};
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        repo(&root);
        std::fs::write(root.join("a.txt"), "a\n").unwrap();
        let base = crate::test_fixtures::commit_all(&root, "a");
        let stream = svc.streams.list_streams().await.unwrap()[0].id.to_string();
        let target = || ChangeTarget::Range {
            stream_id: Some(stream.clone()),
            base: Some(format!("git:{base}")),
            head: "working".into(),
        };
        refresh_files(svc, target()).await.unwrap();
        std::fs::write(root.join("a.txt"), "changed\n").unwrap();
        let paths: std::collections::BTreeSet<String> = ["a.txt".to_string()].into();
        assert!(refresh_paths(svc, target(), &paths).await.unwrap());
        let rows = svc
            .sql
            .query_sql(
                "SELECT f.path, f.status FROM v_change c JOIN v_change_file f ON f.change_id = c.id
                 WHERE c.kind = 'range'",
                Vec::new(),
                None,
            )
            .await
            .unwrap()
            .rows;
        assert_eq!(rows.len(), 1, "{rows:?}");

        std::fs::create_dir(root.join("folder")).unwrap();
        let folder: std::collections::BTreeSet<String> = ["folder".to_string()].into();
        assert!(!refresh_paths(svc, target(), &folder).await.unwrap());
    }

    /// A relist of some paths is followed by one of everything once the stream
    /// is quiet, which finds what no watch reports: `git add` from a terminal.
    #[tokio::test]
    async fn a_relist_by_paths_is_reconciled_once_the_stream_is_quiet() {
        let f = crate::test_fixtures::services_with_effort().await;
        let root = f.svc.layout.project_dir.clone();
        repo(&root);
        std::fs::write(root.join("heard.txt"), "1\n").unwrap();
        std::fs::write(root.join("added.txt"), "1\n").unwrap();
        tokio::spawn(follow_working_files(
            f.svc.clone(),
            Duration::from_millis(400),
        ));
        let staged = |path: &'static str| {
            let svc = f.svc.clone();
            async move {
                working_rows(&svc)
                    .await
                    .into_iter()
                    .find(|row| row[0] == oxplow_db::SqlCell::Text(path.into()))
                    .map(|row| row[4].clone())
            }
        };
        let unstaged = oxplow_db::SqlCell::Int(0);
        let stagedly = oxplow_db::SqlCell::Int(1);
        tokio::time::timeout(Duration::from_secs(30), async {
            while staged("added.txt").await != Some(unstaged.clone()) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("listed at boot");

        let repo_at = git2::Repository::open(&root).unwrap();
        let mut index = repo_at.index().unwrap();
        index.add_path(std::path::Path::new("added.txt")).unwrap();
        index.write().unwrap();
        let stream = f.svc.streams.list_streams().await.unwrap()[0].id;
        f.svc
            .ref_moves
            .paths_moved(stream, ["heard.txt".to_string()]);
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            staged("added.txt").await,
            Some(unstaged),
            "a relist of other paths doesn't look at it"
        );
        tokio::time::timeout(Duration::from_secs(30), async {
            while staged("added.txt").await != Some(stagedly.clone()) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("the whole list, once quiet");
    }

    /// Every stream's working change is there from boot, before anything
    /// is edited, and a stage moves its staged flag with no snapshot.
    #[tokio::test]
    async fn the_working_files_follow_the_index_and_exist_from_boot() {
        let f = crate::test_fixtures::services_with_effort().await;
        let root = f.svc.layout.project_dir.clone();
        repo(&root);
        std::fs::write(root.join("src/new.rs"), "fn n() {}\n").unwrap();
        tokio::spawn(watch_working_files(f.svc.clone()));
        let staged = |path: &'static str| {
            let svc = f.svc.clone();
            async move {
                let out = svc
                    .sql
                    .query_sql(
                        "SELECT f.staged FROM v_change c JOIN v_change_file f ON f.change_id = c.id
                         WHERE c.kind = 'working' AND f.path = ?1",
                        vec![oxplow_db::SqlCell::Text(path.into())],
                        None,
                    )
                    .await
                    .unwrap();
                out.rows.first().map(|r| r[0].clone())
            }
        };
        let wait = |want: Option<oxplow_db::SqlCell>, path: &'static str| {
            let staged = &staged;
            async move {
                for _ in 0..100 {
                    if staged(path).await == want {
                        return true;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                false
            }
        };
        assert!(
            wait(Some(oxplow_db::SqlCell::Int(0)), "src/new.rs").await,
            "listed at boot, unstaged"
        );
        f.svc
            .commands
            .run(
                &oxplow_domain::Actor::Human,
                "oxplow.vcs.stage",
                serde_json::json!({
                    "stream": f.svc.streams.list_streams().await.unwrap()[0].id.to_string(),
                    "paths": ["src/new.rs"]
                }),
                false,
            )
            .await
            .unwrap();
        assert!(
            wait(Some(oxplow_db::SqlCell::Int(1)), "src/new.rs").await,
            "a stage marks it staged"
        );
    }

    /// A take that recorded files lists the working tree's files
    /// at once and paces the deep analysis (`pending_run`, core's job):
    /// it runs once settled, stamping what its inputs had seen.
    #[tokio::test]
    async fn a_snapshot_lists_files_and_paces_the_deep_analysis() {
        let f = crate::test_fixtures::services_with_effort().await;
        repo(&f.svc.layout.project_dir);
        let reactor = ChangeReactor::new(Arc::downgrade(&f.svc));
        let ev = log(&f.svc, taken(false, 1)).await;
        assert!(reactor.handles(&ev.envelope.event_type));
        reactor.handle(&ev).await.unwrap();
        assert_eq!(
            working_paths(&f.svc).await,
            serde_json::json!([["src/lib.rs"]])
        );
        let deep = |svc: Arc<Services>| async move {
            let out = svc
                .sql
                .query_sql(
                    "SELECT events_to IS NOT NULL, (SELECT count(*) FROM v_pending_run WHERE owner = 'core')
                     FROM v_change WHERE kind = 'working'",
                    vec![],
                    None,
                )
                .await
                .unwrap();
            serde_json::to_value(out.rows).unwrap()
        };
        assert_eq!(
            deep(f.svc.clone()).await,
            serde_json::json!([[0, 1]]),
            "paced"
        );
        let later = oxplow_domain::Timestamp::from_unix_ms(
            oxplow_domain::Timestamp::now().unix_ms() + 21_000,
        );
        assert_eq!(crate::pacing::run_due(&f.svc, later).await.unwrap(), 1);
        assert_eq!(
            deep(f.svc.clone()).await,
            serde_json::json!([[1, 0]]),
            "ran and cleared"
        );
    }

    /// A turn's end take that recorded files analyzes at once — the
    /// moment a person looks at what the turn did — and clears the paced
    /// job. The turn's end itself analyzes nothing: its take says whether
    /// anything changed.
    #[tokio::test]
    async fn a_turn_end_take_analyzes_at_once() {
        let f = crate::test_fixtures::services_with_effort().await;
        repo(&f.svc.layout.project_dir);
        let reactor = ChangeReactor::new(Arc::downgrade(&f.svc));
        assert!(!reactor.handles("agent.turn.ended"));
        reactor
            .handle(
                &log(
                    &f.svc,
                    taken_by(oxplow_domain::snapshot::SnapshotTrigger::TurnEnd, false, 1),
                )
                .await,
            )
            .await
            .unwrap();
        let out = f
            .svc
            .sql
            .query_sql(
                "SELECT events_to IS NOT NULL, (SELECT count(*) FROM v_pending_run)
                 FROM v_change WHERE kind = 'working'",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            serde_json::to_value(out.rows).unwrap(),
            serde_json::json!([[1, 0]])
        );
    }

    /// An unchanged take moves nothing.
    #[tokio::test]
    async fn an_unchanged_take_does_nothing() {
        let f = crate::test_fixtures::services_with_effort().await;
        repo(&f.svc.layout.project_dir);
        let reactor = ChangeReactor::new(Arc::downgrade(&f.svc));
        reactor
            .handle(&log(&f.svc, taken(true, 0)).await)
            .await
            .unwrap();
        assert_eq!(working_paths(&f.svc).await, serde_json::json!([]));
    }

    /// A burst analyzes once: an event a newer one supersedes is skipped.
    #[tokio::test]
    async fn a_burst_computes_once() {
        let f = crate::test_fixtures::services_with_effort().await;
        repo(&f.svc.layout.project_dir);
        let reactor = ChangeReactor::new(Arc::downgrade(&f.svc));
        let first = log(&f.svc, taken(false, 1)).await;
        let second = log(&f.svc, taken(false, 2)).await;
        // An unchanged take after them supersedes nothing.
        log(&f.svc, taken(true, 0)).await;
        reactor.handle(&first).await.unwrap();
        assert_eq!(
            working_paths(&f.svc).await,
            serde_json::json!([]),
            "superseded"
        );
        reactor.handle(&second).await.unwrap();
        assert_eq!(
            working_paths(&f.svc).await,
            serde_json::json!([["src/lib.rs"]])
        );
    }

    /// An open effort on the stream is recomputed too — from its start
    /// snapshot to the working tree.
    #[tokio::test]
    async fn a_snapshot_recomputes_open_efforts() {
        let f = crate::test_fixtures::services_with_effort().await;
        let root = f.svc.layout.project_dir.clone();
        repo(&root);
        let capture = crate::snapshot_capture::SnapshotCaptureService::new(
            f.svc.snapshot_store.clone(),
            f.svc.blobs.clone(),
            root.clone(),
            Arc::new(crate::vcs::GitProvider),
            StreamId::new(1),
            1_000_000,
            oxplow_fs_watch::WorkspaceFilter::default(),
        )
        .with_settle_duration(std::time::Duration::ZERO)
        .with_predrain_delay(std::time::Duration::ZERO);
        capture.mark_dirty(
            root.join("src/lib.rs"),
            oxplow_fs_watch::WatchEventKind::Other,
        );
        let start = capture
            .request_snapshot(crate::snapshot_capture::TakeRequest {
                trigger: oxplow_domain::snapshot::SnapshotTrigger::Manual,
                thread_id: None,
                turn_id: None,
                effort_id: None,
                budget: None,
                provider: None,
            })
            .await
            .unwrap()
            .unwrap();
        f.svc
            .effort_store
            .set_start_snapshot(&f.effort, start)
            .await
            .unwrap();
        std::fs::write(root.join("src/new.rs"), "fn n() {}\n").unwrap();
        // The effort's own file: its change is limited to those.
        oxplow_db::EffortStore::record_file(
            &*f.svc.effort_store,
            &f.effort,
            "src/new.rs",
            oxplow_db::EffortFileChange::Created,
            oxplow_db::effort_store::FileRefVersion {
                local_snapshot_id: 0,
                closest_vcs_rev: None,
                vcs_rev_exact: false,
            },
        )
        .await
        .unwrap();
        let reactor = ChangeReactor::new(Arc::downgrade(&f.svc));
        reactor
            .handle(&log(&f.svc, taken(false, 1)).await)
            .await
            .unwrap();
        let out = f
            .svc
            .sql
            .query_sql(
                "SELECT f.path FROM v_change c JOIN v_change_file f ON f.change_id = c.id
                 WHERE c.kind = 'effort' ORDER BY f.path",
                vec![],
                None,
            )
            .await
            .unwrap();
        let paths = serde_json::to_value(out.rows).unwrap();
        assert!(
            paths
                .as_array()
                .unwrap()
                .contains(&serde_json::json!(["src/new.rs"])),
            "{paths}"
        );
    }

    /// P7 review (tsk710): an effort that edits and finishes before any take
    /// while it is open is recomputed against its end snapshot when
    /// `effort.finished` arrives — what `effort_churn` (`after:
    /// [change.analyze]`) reads.
    #[tokio::test]
    async fn a_finished_effort_is_recomputed_against_its_end_snapshot() {
        let f = crate::test_fixtures::services_with_effort().await;
        let root = f.svc.layout.project_dir.clone();
        repo(&root);
        let capture = crate::snapshot_capture::SnapshotCaptureService::new(
            f.svc.snapshot_store.clone(),
            f.svc.blobs.clone(),
            root.clone(),
            Arc::new(crate::vcs::GitProvider),
            StreamId::new(1),
            1_000_000,
            oxplow_fs_watch::WorkspaceFilter::default(),
        )
        .with_settle_duration(std::time::Duration::ZERO)
        .with_predrain_delay(std::time::Duration::ZERO);
        let take = |path: &str| {
            capture.mark_dirty(root.join(path), oxplow_fs_watch::WatchEventKind::Other);
            capture.request_snapshot(crate::snapshot_capture::TakeRequest {
                trigger: oxplow_domain::snapshot::SnapshotTrigger::Manual,
                thread_id: None,
                turn_id: None,
                effort_id: None,
                budget: None,
                provider: None,
            })
        };
        let start = take("src/lib.rs").await.unwrap().unwrap();
        f.svc
            .effort_store
            .set_start_snapshot(&f.effort, start)
            .await
            .unwrap();
        // Analyzed once while open, before the edit.
        let reactor = ChangeReactor::new(Arc::downgrade(&f.svc));
        reactor
            .handle(&log(&f.svc, taken(false, 1)).await)
            .await
            .unwrap();
        // The edit lands and the effort closes: its end snapshot holds it.
        std::fs::write(root.join("src/late.rs"), "fn late() {}\n").unwrap();
        oxplow_db::EffortStore::record_file(
            &*f.svc.effort_store,
            &f.effort,
            "src/late.rs",
            oxplow_db::EffortFileChange::Created,
            oxplow_db::effort_store::FileRefVersion {
                local_snapshot_id: 0,
                closest_vcs_rev: None,
                vcs_rev_exact: false,
            },
        )
        .await
        .unwrap();
        let end = take("src/late.rs").await.unwrap().unwrap();
        f.svc
            .effort_store
            .set_end_snapshot(&f.effort, end)
            .await
            .unwrap();
        let effort = f.effort;
        f.svc
            .db
            .transaction(move |c| {
                c.execute(
                    "UPDATE effort SET ended_at = '2026-10-02T00:00:00.000000Z' WHERE id = ?1",
                    [effort.value()],
                )
                .map(|_| ())
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert!(reactor.handles("effort.finished"));
        let finished = Envelope::typed::<oxplow_domain::events::schema::EffortFinished>(
            "system",
            &oxplow_domain::events::schema::EffortFinishedV3 {
                effort: f.effort.to_string(),
                work_item: Some("oxplow:oxplow/work_item?id=tsk1&provider=oxplow".into()),
                end_snapshot: Some(end),
            },
        );
        reactor.handle(&log(&f.svc, finished).await).await.unwrap();
        let out = f
            .svc
            .sql
            .query_sql(
                "SELECT f.path FROM v_change c JOIN v_change_file f ON f.change_id = c.id
                 WHERE c.kind = 'effort' ORDER BY f.path",
                vec![],
                None,
            )
            .await
            .unwrap();
        let paths = serde_json::to_value(out.rows).unwrap();
        assert!(
            paths
                .as_array()
                .unwrap()
                .contains(&serde_json::json!(["src/late.rs"])),
            "{paths}"
        );
    }

    /// An effort that finished with no start snapshot (opened and closed
    /// at once, after the fact) has no change to analyze: its
    /// `effort.finished` is done, not a dead letter and an alert.
    #[tokio::test]
    async fn a_finished_effort_with_no_start_snapshot_is_skipped() {
        let f = crate::test_fixtures::services_with_effort().await;
        let reactor = ChangeReactor::new(Arc::downgrade(&f.svc));
        let finished = Envelope::typed::<oxplow_domain::events::schema::EffortFinished>(
            "system",
            &oxplow_domain::events::schema::EffortFinishedV3 {
                effort: f.effort.to_string(),
                work_item: None,
                end_snapshot: None,
            },
        );
        reactor.handle(&log(&f.svc, finished).await).await.unwrap();
    }

    /// A failed analysis is an error naming the stream (the pump makes it a
    /// dead letter).
    #[tokio::test]
    async fn a_failed_analysis_names_the_stream() {
        let f = crate::test_fixtures::services_with_effort().await;
        std::fs::remove_dir_all(f.svc.layout.project_dir.join(".git")).unwrap();
        let reactor = ChangeReactor::new(Arc::downgrade(&f.svc));
        let err = reactor
            .handle(&log(&f.svc, taken(false, 1)).await)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("analyzing stream str1"), "{err}");
    }

    /// HEAD moving recomputes the working change.
    /// A range whose head is the working tree moves with the stream: a
    /// head move lists its files again, as it does the `working` change's.
    /// One between two fixed revisions is left as computed.
    #[tokio::test]
    async fn a_head_move_refreshes_the_ranges_headed_at_the_working_tree() {
        let f = crate::test_fixtures::services_with_effort().await;
        let root = f.svc.layout.project_dir.clone();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/lib.rs"), BEFORE).unwrap();
        let base = crate::test_fixtures::commit_all(&root, "base");
        std::fs::write(root.join("src/lib.rs"), "fn a() -> i32 {\n    2\n}\n").unwrap();
        let head = crate::test_fixtures::commit_all(&root, "change");
        let range = |head: &str| ChangeTarget::Range {
            stream_id: Some("str1".into()),
            base: Some(format!("git:{base}")),
            head: head.to_string(),
        };
        let moving = crate::change_analysis::ensure_change(&f.svc, range("working"))
            .await
            .unwrap();
        let fixed = crate::change_analysis::ensure_change(&f.svc, range(&format!("git:{head}")))
            .await
            .unwrap();
        std::fs::write(root.join("src/new.rs"), "fn n() {}\n").unwrap();
        let reactor = ChangeReactor::new(Arc::downgrade(&f.svc));
        reactor
            .handle(&log(&f.svc, head_moved()).await)
            .await
            .unwrap();
        let svc = &f.svc;
        let paths = |id: i64| async move {
            let out = svc
                .sql
                .query_sql(
                    "SELECT path FROM v_change_file WHERE change_id = ?1 ORDER BY path",
                    vec![oxplow_db::SqlCell::Int(id)],
                    None,
                )
                .await
                .unwrap();
            serde_json::to_value(out.rows).unwrap()
        };
        assert_eq!(
            paths(moving.id).await,
            serde_json::json!([["src/lib.rs"], ["src/new.rs"]])
        );
        assert_eq!(paths(fixed.id).await, serde_json::json!([["src/lib.rs"]]));
        // Its deep analysis is paced under a job that names it back.
        let job = job_id(&range("working")).unwrap();
        assert_eq!(job, format!("change/range/str1/git:{base}..working"));
        assert_eq!(target_of(&job), Some(range("working")));
    }

    #[tokio::test]
    async fn a_head_move_recomputes_the_working_change() {
        let f = crate::test_fixtures::services_with_effort().await;
        repo(&f.svc.layout.project_dir);
        let reactor = ChangeReactor::new(Arc::downgrade(&f.svc));
        reactor
            .handle(&log(&f.svc, head_moved()).await)
            .await
            .unwrap();
        assert_eq!(
            working_paths(&f.svc).await,
            serde_json::json!([["src/lib.rs"]])
        );
    }
}
