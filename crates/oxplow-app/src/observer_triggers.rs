//! Triggers for observers (`.context/extensions-jobs.md` "Triggers for
//! observers"): what runs a collector whose `on:` lists `start`,
//! `{ files: <glob> }` or `{ sqlite: own | <path> }` beside, or instead of,
//! event types. A store's collector follows its tool this way, with no
//! oxplow event in between.
//!
//! One task per observing collector (so its runs never overlap) fed by one
//! small watcher per source:
//!
//! - `start`: once when the collector is first seen on, which is boot, an
//!   enable, or a deploy that changed it.
//! - `files`: the primary stream's watch ([`StreamWatchRegistry::subscribe_primary`])
//!   narrowed to the glob's literal folder; changes are coalesced over the
//!   assets window ([`crate::assets::COALESCE`], at most
//!   [`crate::assets::MAX_WAIT_FACTOR`] windows) into one run.
//! - `sqlite`: `PRAGMA data_version` polled every [`POLL`] on one read
//!   connection kept for the file (the number is only comparable on the same
//!   connection).
//!
//! [`supervise`] keeps the set in step with the catalog: a collector whose
//! extension is off (or gone) has no watcher and never runs; one that
//! changed starts over. A run that fails keeps its cursor
//! (`collector_run.cursor_json`) and the next trigger runs it again.
//!
//! [`StreamWatchRegistry::subscribe_primary`]: crate::stream_watch::StreamWatchRegistry::subscribe_primary

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use oxplow_config::collectors::ObserverSource;
use oxplow_fs_watch::{Batch, Scope, Subscription, WatchEventKind};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::assets::{COALESCE, MAX_WAIT_FACTOR};
use crate::collector_runner::{FileChange, FileOp, RunTrigger};
use crate::Services;

/// How often a SQLite file's `data_version` is read.
pub const POLL: Duration = Duration::from_secs(2);

/// The folder a glob lives in: its leading parts with no wildcard
/// (`shelf/*.md` → `shelf`), joined by `/`; empty for a glob that starts with
/// one (`**/*.md`).
pub fn literal_dir(glob: &str) -> String {
    glob.split('/')
        .take_while(|part| !part.contains(['*', '?', '[', '{']))
        .collect::<Vec<_>>()
        .join("/")
}

/// What of the worktree the globs need to hear: each glob's literal folder,
/// or every kept path when one has none.
pub fn scope_of(globs: &[String]) -> Scope {
    let mut scope = Scope::default();
    for glob in globs {
        let dir = literal_dir(glob);
        scope = if dir.is_empty() {
            scope.all_kept()
        } else {
            scope.dir(dir)
        };
    }
    scope
}

pub use oxplow_config::collectors::files_matcher as matcher;

fn op_of(kind: &WatchEventKind) -> FileOp {
    match kind {
        WatchEventKind::Created => FileOp::Created,
        WatchEventKind::Removed => FileOp::Removed,
        WatchEventKind::Modified | WatchEventKind::Other => FileOp::Modified,
    }
}

/// `path` from the project root, `/`-separated.
fn relative(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    Some(
        rel.components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
    )
}

/// The files under `dir` that exist, for a rescan (changes the watch lost):
/// each is reported as modified. A removal lost with them is not found.
fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.file_name().is_some_and(|n| n == ".git") {
            continue;
        }
        match entry.file_type() {
            Ok(t) if t.is_dir() => files_under(&path, out),
            Ok(t) if t.is_file() => out.push(path),
            _ => {}
        }
    }
}

/// What a batch changed that `glob` matches, as the collector sees it.
pub fn changes_in(root: &Path, glob: &globset::GlobSet, batch: &Batch) -> Vec<FileChange> {
    let mut out: Vec<FileChange> = Vec::new();
    let mut add = |path: &Path, op: FileOp| {
        if let Some(rel) = relative(root, path).filter(|rel| glob.is_match(rel)) {
            out.push(FileChange { path: rel, op });
        }
    };
    for (path, kind) in &batch.paths {
        add(path, op_of(kind));
    }
    for dir in &batch.rescans {
        let mut found = Vec::new();
        files_under(dir, &mut found);
        for path in found {
            if !batch.paths.iter().any(|(p, _)| p == &path) {
                add(&path, FileOp::Modified);
            }
        }
    }
    out
}

/// Fold `later` changes into `earlier` ones: a path keeps one entry, the
/// last thing that happened to it. An entry still `Created` is a path that
/// did not exist before the burst (its first op was `created`), so created
/// and then removed never was and leaves no entry. A path that existed
/// before (first op `modified` or `removed`) and is removed keeps a
/// `removed`: removed then created is `modified` (it exists, and changed),
/// so a later removal is not lost.
pub fn merge(earlier: &mut Vec<FileChange>, later: Vec<FileChange>) {
    for change in later {
        match earlier.iter().position(|c| c.path == change.path) {
            None => earlier.push(change),
            Some(at) => {
                let was = earlier[at].op;
                match (was, change.op) {
                    (FileOp::Created, FileOp::Removed) => {
                        earlier.remove(at);
                    }
                    (FileOp::Created, FileOp::Modified) => {}
                    (FileOp::Removed, FileOp::Created) => earlier[at].op = FileOp::Modified,
                    // It existed before the burst: a `created` over it is a
                    // rewrite, and a removal after still has to be heard.
                    (FileOp::Modified, FileOp::Created) => {}
                    (_, op) => earlier[at].op = op,
                }
            }
        }
    }
}

/// Hear `changes` for as long as they come and send what matches `glob`,
/// one message per burst: after the first change it waits for [`COALESCE`]
/// of quiet, but no longer than [`MAX_WAIT_FACTOR`] windows in all.
pub async fn follow_files(
    mut changes: Subscription,
    root: PathBuf,
    glob: globset::GlobSet,
    send: mpsc::UnboundedSender<RunTrigger>,
) {
    while let Some(first) = changes.next_batch().await {
        let mut found = changes_in(&root, &glob, &first);
        // A rescan can't see a removal: the burst can't be trusted to name
        // everything, so it asks for the full reconcile.
        let mut rescanned = !first.rescans.is_empty();
        let deadline = tokio::time::Instant::now() + COALESCE * MAX_WAIT_FACTOR;
        loop {
            let window =
                COALESCE.min(deadline.saturating_duration_since(tokio::time::Instant::now()));
            if window.is_zero() {
                break;
            }
            match tokio::time::timeout(window, changes.next_batch()).await {
                Ok(Some(more)) => {
                    rescanned |= !more.rescans.is_empty();
                    merge(&mut found, changes_in(&root, &glob, &more));
                }
                // Quiet for the window (or the watch ended): the burst is over.
                Ok(None) | Err(_) => break,
            }
        }
        let trigger = if rescanned {
            Some(RunTrigger::Start)
        } else if found.is_empty() {
            None
        } else {
            Some(RunTrigger::Files(found))
        };
        if trigger.is_some_and(|t| send.send(t).is_err()) {
            return;
        }
    }
}

/// `PRAGMA data_version` of `conn`.
fn data_version(conn: &rusqlite::Connection) -> rusqlite::Result<i64> {
    conn.query_row("PRAGMA data_version", [], |r| r.get(0))
}

/// A read connection to a SQLite file and its `data_version` when opened.
pub type Baseline = (rusqlite::Connection, i64);

/// Open `path` read-only and read its `data_version`: the baseline writes
/// are compared with. None when the file isn't there or can't be read.
pub fn open_baseline(path: &Path) -> Option<Baseline> {
    if !path.is_file() {
        return None;
    }
    rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .ok()
    .and_then(|c| data_version(&c).ok().map(|v| (c, v)))
}

/// Poll the SQLite file at `path` every `every` and send when another
/// connection wrote to it. The connection is opened once the file exists and
/// kept: `data_version` is only comparable on one connection. The first read
/// is the baseline, not a change.
///
/// `baseline` is the connection and `data_version` read before the `start`
/// run (see [`open_baseline`]), so a write made after that read, while the
/// run goes on, is heard. None: the first read the poll makes is the
/// baseline.
pub async fn poll_sqlite(
    path: PathBuf,
    every: Duration,
    send: mpsc::UnboundedSender<RunTrigger>,
    baseline: Option<Baseline>,
) {
    let mut conn: Option<Baseline> = baseline;
    loop {
        tokio::time::sleep(every).await;
        if send.is_closed() {
            return;
        }
        let Some((open, last)) = conn.as_mut() else {
            if path.is_file() {
                conn = open_baseline(&path);
            }
            continue;
        };
        match data_version(open) {
            Ok(now) if now != *last => {
                *last = now;
                if send.send(RunTrigger::Sqlite).is_err() {
                    return;
                }
            }
            Ok(_) => {}
            // The file went away or broke: open it again when it is back.
            Err(_) => conn = None,
        }
    }
}

/// An observing collector as the supervisor sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observer {
    /// The namespace it is named in.
    pub owner: String,
    pub id: String,
    /// The extension's own folder name: its SQLite file is named for it.
    pub extension: String,
    pub sources: Vec<ObserverSource>,
    /// Changes when the collector, or the deployment it runs from, does.
    pub stamp: String,
}

/// The collectors that observe, in extensions that are on.
pub fn observers(svc: &Services) -> Vec<Observer> {
    let live = svc.extension_catalog.live();
    svc.extension_catalog
        .unpinned(
            &svc.layout.project_dir,
            "a trigger is read from declarations; the run it starts pins",
        )
        .iter()
        .filter(|ext| ext.enabled)
        .flat_map(|ext| {
            let deployed = live.get(&ext.name).cloned().unwrap_or_default();
            ext.collectors
                .iter()
                .filter(|c| c.facts.is_empty() && !c.trigger.sources().is_empty())
                .map(move |c| Observer {
                    owner: ext.namespace.clone(),
                    id: c.id.clone(),
                    extension: ext.name.clone(),
                    sources: c.trigger.sources().to_vec(),
                    stamp: format!("{deployed}/{:?}", c),
                })
        })
        .collect()
}

/// What is running for one observer: its task and the watchers feeding it.
struct Running {
    stamp: String,
    tasks: Vec<JoinHandle<()>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Run `observer`'s collector for each trigger that comes, one at a time;
/// the triggers that piled up behind a run fold into one.
async fn run_triggers(
    svc: std::sync::Weak<Services>,
    observer: Observer,
    mut triggers: mpsc::UnboundedReceiver<RunTrigger>,
) {
    // Set when a run failed: what it was told of is not known to be stored
    // (its `changed` list is gone), so the next run reconciles in full.
    let mut needs_reconcile = false;
    while let Some(mut trigger) = triggers.recv().await {
        while let Ok(more) = triggers.try_recv() {
            trigger = fold(trigger, more);
        }
        if needs_reconcile {
            trigger = RunTrigger::Start;
        }
        let Some(svc) = svc.upgrade() else { return };
        needs_reconcile = !fire(&svc, &observer, trigger).await;
    }
}

/// Two triggers that wait for the same run: the files add up, two of a
/// kind stay that kind, and two different kinds become `start`, the full
/// reconcile, so neither's changes are lost.
fn fold(first: RunTrigger, second: RunTrigger) -> RunTrigger {
    match (first, second) {
        (RunTrigger::Files(mut a), RunTrigger::Files(b)) => {
            merge(&mut a, b);
            RunTrigger::Files(a)
        }
        (RunTrigger::Sqlite, RunTrigger::Sqlite) => RunTrigger::Sqlite,
        // `start` is the full reconcile: nothing after it adds to it.
        _ => RunTrigger::Start,
    }
}

/// Run `observer`'s collector for `trigger`, if it is still on. False when
/// the run failed or couldn't run: the caller reconciles in full next.
pub async fn fire(svc: &Arc<Services>, observer: &Observer, trigger: RunTrigger) -> bool {
    if !observers(svc)
        .iter()
        .any(|o| o.owner == observer.owner && o.id == observer.id)
    {
        return true;
    }
    let root = svc.layout.project_dir.clone();
    let ctx = crate::collector_runner::Collectors::of(svc, &root);
    let run = oxplow_domain::run::RunContext::core(
        oxplow_domain::Actor::System,
        oxplow_domain::run::Mode::External,
    );
    let name = trigger.name();
    let ran = svc
        .sql
        .read_generation(crate::collector_runner::run_collector(
            &ctx,
            &run,
            &observer.owner,
            &observer.id,
            trigger,
        ))
        .await;
    match ran {
        Ok(Ok(_)) => true,
        Ok(Err(error)) => {
            tracing::warn!(
                collector = %format!("{}/{}", observer.owner, observer.id),
                trigger = name,
                error = ?error,
                "an observing collector failed; its cursor is kept and the next run reconciles in full"
            );
            false
        }
        Err(error) => {
            tracing::warn!(?error, "an observing collector couldn't run");
            false
        }
    }
}

/// Start `observer`'s task and watchers.
fn start(svc: &Arc<Services>, observer: &Observer) -> Running {
    let (send, receive) = mpsc::unbounded_channel();
    let mut tasks = vec![tokio::spawn(run_triggers(
        Arc::downgrade(svc),
        observer.clone(),
        receive,
    ))];
    let root = svc.layout.project_dir.clone();
    let globs: Vec<String> = observer
        .sources
        .iter()
        .filter_map(|s| match s {
            ObserverSource::Files { glob } => Some(glob.clone()),
            _ => None,
        })
        .collect();
    if !globs.is_empty() {
        match matcher(&globs) {
            Ok(set) => tasks.push(tokio::spawn(follow_files(
                svc.stream_watches.subscribe_primary(scope_of(&globs)),
                root.clone(),
                set,
                send.clone(),
            ))),
            Err(e) => tracing::warn!(error = %e, "a files trigger isn't a glob"),
        }
    }
    // The SQLite baselines are read before the `start` run is sent, so a
    // write while that run goes on is a change the poll hears.
    for source in &observer.sources {
        if let ObserverSource::Sqlite { path } = source {
            let file = match path {
                None => {
                    crate::extension::extension_services::store_path(&root, &observer.extension)
                }
                Some(path) => root.join(path),
            };
            let baseline = open_baseline(&file);
            tasks.push(tokio::spawn(poll_sqlite(
                file,
                POLL,
                send.clone(),
                baseline,
            )));
        }
    }
    if observer
        .sources
        .iter()
        .any(|s| matches!(s, ObserverSource::Start))
    {
        let _ = send.send(RunTrigger::Start);
    }
    Running {
        stamp: observer.stamp.clone(),
        tasks,
    }
}

/// Keep the running observers in step with `wanted`: start the ones that
/// are new or changed, stop the ones that are gone (their extension off).
fn reconcile(
    svc: &Arc<Services>,
    running: &mut BTreeMap<(String, String), Running>,
    wanted: Vec<Observer>,
) {
    running.retain(|key, have| {
        wanted
            .iter()
            .any(|o| (&o.owner, &o.id) == (&key.0, &key.1) && o.stamp == have.stamp)
    });
    for observer in wanted {
        running
            .entry((observer.owner.clone(), observer.id.clone()))
            .or_insert_with(|| start(svc, &observer));
    }
}

/// Follow the catalog for as long as `svc` lives: observers start when their
/// extension is on and stop when it isn't. Spawned once at boot, when the
/// watches are up.
pub fn supervise(svc: &Arc<Services>) -> JoinHandle<()> {
    let weak = Arc::downgrade(svc);
    let (mut changes, mut runtime) = (
        svc.extension_catalog.changes(),
        svc.extension_catalog.runtime_changes(),
    );
    tokio::spawn(async move {
        let mut running: BTreeMap<(String, String), Running> = BTreeMap::new();
        loop {
            let Some(svc) = weak.upgrade() else { return };
            reconcile(&svc, &mut running, observers(&svc));
            drop(svc);
            // A change, or a look now and then in case one was missed.
            tokio::select! {
                _ = changes.recv() => {}
                _ = runtime.recv() => {}
                () = tokio::time::sleep(Duration::from_secs(30)) => {}
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_fs_watch::Feed;
    use serde_json::json;

    fn files(globs: &[&str]) -> globset::GlobSet {
        matcher(&globs.iter().map(|g| g.to_string()).collect::<Vec<_>>()).unwrap()
    }

    fn created(path: &str) -> FileChange {
        FileChange {
            path: path.into(),
            op: FileOp::Created,
        }
    }

    #[test]
    fn a_globs_folder_is_what_the_watch_is_narrowed_to() {
        assert_eq!(literal_dir("shelf/*.md"), "shelf");
        assert_eq!(literal_dir("a/b/**/*.md"), "a/b");
        assert_eq!(literal_dir("notes.md"), "notes.md");
        assert_eq!(literal_dir("**/*.md"), "");
        assert_eq!(literal_dir("*.md"), "");
    }

    /// `*` stays inside a folder; `**` goes down.
    #[test]
    fn a_star_stays_in_its_folder() {
        let set = files(&["shelf/*.md"]);
        assert!(set.is_match("shelf/a.md"));
        assert!(!set.is_match("shelf/deep/a.md"));
        assert!(files(&["shelf/**/*.md"]).is_match("shelf/deep/a.md"));
    }

    /// A path keeps its last state; created then changed is still created,
    /// created then removed never was.
    #[test]
    fn merging_keeps_each_paths_last_state() {
        let mut found = vec![created("a.md"), created("b.md")];
        merge(
            &mut found,
            vec![
                FileChange {
                    path: "a.md".into(),
                    op: FileOp::Modified,
                },
                FileChange {
                    path: "b.md".into(),
                    op: FileOp::Removed,
                },
                created("c.md"),
            ],
        );
        assert_eq!(found, vec![created("a.md"), created("c.md")]);
    }

    fn change(path: &str, op: FileOp) -> FileChange {
        FileChange {
            path: path.into(),
            op,
        }
    }

    /// A removal is never lost: created then removed is nothing only when
    /// the path did not exist before the burst. A path that did (its first op
    /// was a removal or a change, or an atomic write showed as `created`
    /// over it) still ends as removed.
    #[test]
    fn a_removal_of_a_path_that_existed_is_kept() {
        let run = |ops: &[FileOp]| {
            let mut found = Vec::new();
            for op in ops {
                merge(&mut found, vec![change("a.md", *op)]);
            }
            found
        };
        use FileOp::{Created, Modified, Removed};
        assert_eq!(run(&[Removed, Created, Removed]), [change("a.md", Removed)]);
        assert_eq!(
            run(&[Modified, Created, Removed]),
            [change("a.md", Removed)]
        );
        assert_eq!(run(&[Removed, Created]), [change("a.md", Modified)]);
        assert_eq!(run(&[Created, Removed]), Vec::<FileChange>::new());
        assert_eq!(run(&[Created, Removed, Created]), [change("a.md", Created)]);
    }

    /// Triggers that wait for one run: files add up; two kinds are the full
    /// reconcile, so the files' changes are not replaced by a SQLite one.
    #[test]
    fn folding_different_kinds_is_a_start() {
        let a = RunTrigger::Files(vec![created("a.md")]);
        assert!(matches!(
            fold(a.clone(), RunTrigger::Sqlite),
            RunTrigger::Start
        ));
        assert!(matches!(
            fold(RunTrigger::Sqlite, a.clone()),
            RunTrigger::Start
        ));
        assert!(matches!(
            fold(RunTrigger::Sqlite, RunTrigger::Sqlite),
            RunTrigger::Sqlite
        ));
        assert!(matches!(
            fold(RunTrigger::Start, a.clone()),
            RunTrigger::Start
        ));
        match fold(a, RunTrigger::Files(vec![created("b.md")])) {
            RunTrigger::Files(found) => assert_eq!(found.len(), 2),
            other => panic!("files, got {other:?}"),
        }
    }

    /// A rescan can't see a removal: the burst asks for a `start` run.
    #[tokio::test(start_paused = true)]
    async fn a_rescan_is_a_start_run() {
        let feed = Feed::default();
        let (send, mut receive) = mpsc::unbounded_channel();
        let root = PathBuf::from("/p");
        tokio::spawn(follow_files(
            feed.subscribe(),
            root.clone(),
            files(&["shelf/*.md"]),
            send,
        ));
        feed.rescan(&root.join("shelf"));
        assert!(matches!(receive.recv().await, Some(RunTrigger::Start)));
    }

    /// A write made after the baseline was read, before the poll's first
    /// look, is heard (the baseline is read before the `start` run).
    #[tokio::test]
    async fn a_write_after_the_baseline_but_before_the_first_poll_is_heard() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tool.sqlite");
        let writer = rusqlite::Connection::open(&file).unwrap();
        writer
            .execute_batch("CREATE TABLE t (x); INSERT INTO t VALUES (1);")
            .unwrap();
        let baseline = open_baseline(&file);
        assert!(baseline.is_some());
        writer.execute_batch("INSERT INTO t VALUES (2);").unwrap();
        let (send, mut receive) = mpsc::unbounded_channel();
        let poll = tokio::spawn(poll_sqlite(file, Duration::from_millis(20), send, baseline));
        let heard = tokio::time::timeout(Duration::from_secs(5), receive.recv())
            .await
            .expect("heard in time");
        assert!(matches!(heard, Some(RunTrigger::Sqlite)));
        poll.abort();
    }

    async fn heard(receive: &mut mpsc::UnboundedReceiver<RunTrigger>) -> Vec<FileChange> {
        match receive.recv().await {
            Some(RunTrigger::Files(changed)) => changed,
            other => panic!("a files trigger, got {other:?}"),
        }
    }

    /// A burst of changes under the window is one run; a change after the
    /// quiet is another; a file the glob doesn't match runs nothing.
    #[tokio::test(start_paused = true)]
    async fn a_burst_of_file_changes_runs_the_collector_once() {
        let feed = Feed::default();
        let (send, mut receive) = mpsc::unbounded_channel();
        let root = PathBuf::from("/p");
        tokio::spawn(follow_files(
            feed.subscribe(),
            root.clone(),
            files(&["shelf/*.md"]),
            send,
        ));
        let at = |rel: &str| root.join(rel);
        feed.change(&at("shelf/a.md"), &WatchEventKind::Created);
        feed.change(&at("other/x.md"), &WatchEventKind::Modified);
        tokio::time::sleep(COALESCE / 2).await;
        feed.change(&at("shelf/b.md"), &WatchEventKind::Modified);
        let mut first = heard(&mut receive).await;
        first.sort_by(|a, b| a.path.cmp(&b.path));
        assert_eq!(
            first,
            vec![
                created("shelf/a.md"),
                FileChange {
                    path: "shelf/b.md".into(),
                    op: FileOp::Modified
                }
            ]
        );
        assert!(receive.try_recv().is_err(), "one run for the burst");

        tokio::time::sleep(COALESCE * 2).await;
        feed.change(&at("shelf/a.md"), &WatchEventKind::Removed);
        assert_eq!(
            heard(&mut receive).await,
            vec![FileChange {
                path: "shelf/a.md".into(),
                op: FileOp::Removed
            }]
        );
        // Only what the glob doesn't match: nothing to run.
        feed.change(&at("other/y.md"), &WatchEventKind::Modified);
        tokio::time::sleep(COALESCE * 5).await;
        assert!(receive.try_recv().is_err());
    }

    /// A write that never goes quiet still runs, [`MAX_WAIT_FACTOR`] windows
    /// after the first change.
    #[tokio::test(start_paused = true)]
    async fn a_write_that_never_stops_still_runs() {
        let feed = Feed::default();
        let (send, mut receive) = mpsc::unbounded_channel();
        let root = PathBuf::from("/p");
        tokio::spawn(follow_files(
            feed.subscribe(),
            root.clone(),
            files(&["*.md"]),
            send,
        ));
        let writer = {
            let (feed, path) = (feed.clone(), root.join("a.md"));
            tokio::spawn(async move {
                for _ in 0..40 {
                    feed.change(&path, &WatchEventKind::Modified);
                    tokio::time::sleep(COALESCE / 2).await;
                }
            })
        };
        let started = tokio::time::Instant::now();
        assert_eq!(heard(&mut receive).await.len(), 1);
        assert!(started.elapsed() <= COALESCE * (MAX_WAIT_FACTOR + 1));
        writer.abort();
    }

    /// A rescan (changes the watch lost) reports the matching files on disk.
    #[test]
    fn a_rescan_reports_the_matching_files_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("shelf")).unwrap();
        std::fs::write(dir.path().join("shelf/a.md"), "x").unwrap();
        std::fs::write(dir.path().join("shelf/a.txt"), "x").unwrap();
        let batch = Batch {
            paths: Vec::new(),
            rescans: vec![dir.path().join("shelf")],
        };
        assert_eq!(
            changes_in(dir.path(), &files(&["shelf/*.md"]), &batch),
            vec![FileChange {
                path: "shelf/a.md".into(),
                op: FileOp::Modified
            }]
        );
    }

    /// A write by another connection moves `data_version`; the poll sends
    /// once per write, not for the first look, and finds a file made late.
    #[tokio::test]
    async fn a_write_to_a_sqlite_file_is_heard() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("tool.sqlite");
        let (send, mut receive) = mpsc::unbounded_channel();
        let poll = tokio::spawn(poll_sqlite(
            file.clone(),
            Duration::from_millis(20),
            send,
            None,
        ));
        // Not there yet: nothing, and no failure.
        tokio::time::sleep(Duration::from_millis(60)).await;
        let writer = rusqlite::Connection::open(&file).unwrap();
        writer
            .execute_batch("CREATE TABLE t (x); INSERT INTO t VALUES (1);")
            .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(receive.try_recv().is_err(), "the first look is a baseline");
        writer.execute_batch("INSERT INTO t VALUES (2);").unwrap();
        let heard = tokio::time::timeout(Duration::from_secs(5), receive.recv())
            .await
            .expect("heard in time");
        assert!(matches!(heard, Some(RunTrigger::Sqlite)));
        poll.abort();
    }

    /// A `{ files: ".oxplow/oxwiki/*.md" }` trigger is heard on a real
    /// workspace watch although `.oxplow/` ignores itself (`.oxplow/.gitignore`
    /// is `*`: a page in a folder that was there when the watch began.
    #[tokio::test]
    async fn a_trigger_on_oxwiki_pages_is_heard_though_oxplow_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join(".oxplow/oxwiki")).unwrap();
        std::fs::write(root.join(".oxplow/.gitignore"), "*\n").unwrap();
        let watch = oxplow_fs_watch::FsWatcher::watch_workspace(
            &root,
            oxplow_fs_watch::WorkspaceFilter::for_project(
                &root,
                Vec::<String>::new(),
                Vec::<String>::new(),
            ),
        )
        .unwrap();
        let globs = vec![".oxplow/oxwiki/*.md".to_string()];
        let (send, mut receive) = mpsc::unbounded_channel();
        let follow = tokio::spawn(follow_files(
            watch.subscribe_to(scope_of(&globs)),
            root.clone(),
            matcher(&globs).unwrap(),
            send,
        ));
        // Let the watch settle before the first write.
        tokio::time::sleep(Duration::from_millis(500)).await;
        std::fs::write(root.join(".oxplow/oxwiki/home.md"), "# Home").unwrap();
        let heard = tokio::time::timeout(Duration::from_secs(15), receive.recv())
            .await
            .expect("a page in the existing folder is heard");
        match heard {
            Some(RunTrigger::Files(changes)) => {
                assert_eq!(
                    changes.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
                    [".oxplow/oxwiki/home.md"]
                );
            }
            other => panic!("expected a files trigger, got {other:?}"),
        }
        follow.abort();
    }

    const HEAD: &str =
        "manifest: 2\nname: shelf\nintent: { purpose: t, origin: null, examples: [] }\n";

    /// Counts its runs, says which trigger ran it and what it was told, and
    /// keeps a cursor; a files run for `shelf/boom.md` fails.
    const SCRIPT: &str = r#"def transform(x):
    changed = x.get("changed", [])
    if len(changed) > 0 and changed[0]["path"] == "shelf/boom.md":
        fail("boom")
    n = 1
    if x["cursor"] != None:
        n = x["cursor"]["n"] + 1
    names = ",".join([c["path"] + ":" + c["op"] for c in changed])
    return {"entities": {"seen": [{"n": n, "trigger": x["trigger"], "changed": names, "cursor": str(x["cursor"])}]}, "cursor": {"n": n}}
"#;

    const COLLECTOR: &str = "  - id: seen\n    runtime: starlark\n    entry: seen.star\n    on: [start, { files: \"shelf/*.md\" }, { sqlite: own }]\n    sync: upsert\n    entities:\n      - { name: seen, key: n, columns: { n: int, trigger: text, changed: text, cursor: text } }\n";

    async fn declare(svc: &Arc<Services>) {
        let dir = svc.layout.project_dir.join(".oxplow/extensions/shelf");
        std::fs::create_dir_all(dir.join("storage/migrations")).unwrap();
        std::fs::write(
            dir.join("extension.yaml"),
            format!("{HEAD}storage: {{ sqlite: true }}\ncollectors:\n{COLLECTOR}"),
        )
        .unwrap();
        std::fs::write(
            dir.join("storage/migrations/V1__t.sql"),
            "CREATE TABLE t (x);",
        )
        .unwrap();
        std::fs::write(dir.join("seen.star"), SCRIPT).unwrap();
        crate::test_fixtures::activate(svc).await;
    }

    async fn seen(svc: &Services) -> serde_json::Value {
        let out = svc
            .sql
            .query_sql(
                "SELECT n, trigger, changed, cursor FROM v_shelf_seen ORDER BY n",
                vec![],
                None,
            )
            .await
            .unwrap();
        serde_json::to_value(out.rows).unwrap()
    }

    fn observer(svc: &Services) -> Observer {
        observers(svc)
            .into_iter()
            .find(|o| o.id == "seen")
            .expect("the collector observes")
    }

    /// Each trigger runs the collector with its name in `x["trigger"]`, a
    /// files trigger with what changed, and every run with the cursor the
    /// last good run returned. A failed run keeps that cursor.
    #[tokio::test]
    async fn a_trigger_runs_the_collector_with_its_input_and_the_last_cursor() {
        let fx = crate::test_fixtures::services_with_thread().await;
        declare(&fx.svc).await;
        let o = observer(&fx.svc);
        assert_eq!(o.sources.len(), 3);
        fire(&fx.svc, &o, RunTrigger::Start).await;
        fire(&fx.svc, &o, RunTrigger::Files(vec![created("shelf/a.md")])).await;
        // A failed run: nothing is stored, the cursor stays at 2.
        fire(
            &fx.svc,
            &o,
            RunTrigger::Files(vec![created("shelf/boom.md")]),
        )
        .await;
        fire(&fx.svc, &o, RunTrigger::Sqlite).await;
        assert_eq!(
            seen(&fx.svc).await,
            json!([
                [1, "start", "", "None"],
                [2, "files", "shelf/a.md:created", "{\"n\": 1}"],
                [3, "sqlite", "", "{\"n\": 2}"],
            ])
        );
    }

    /// A manual run gets the same keys: `trigger: "manual"` and an empty
    /// `changed`, so a script can treat it like `start`.
    #[tokio::test]
    async fn a_manual_run_is_told_its_trigger_and_no_changes() {
        let fx = crate::test_fixtures::services_with_thread().await;
        declare(&fx.svc).await;
        let o = observer(&fx.svc);
        assert!(fire(&fx.svc, &o, RunTrigger::Manual).await);
        assert_eq!(seen(&fx.svc).await, json!([[1, "manual", "", "None"]]));
    }

    /// After a failed run (its `changed` list is gone) the next run of the
    /// collector is a `start`, whatever trigger came.
    #[tokio::test]
    async fn the_run_after_a_failed_one_is_a_start() {
        let fx = crate::test_fixtures::services_with_thread().await;
        declare(&fx.svc).await;
        let (send, receive) = mpsc::unbounded_channel();
        let task = tokio::spawn(run_triggers(
            Arc::downgrade(&fx.svc),
            observer(&fx.svc),
            receive,
        ));
        send.send(RunTrigger::Start).unwrap();
        wait_for_runs(&fx.svc, 1).await;
        send.send(RunTrigger::Files(vec![created("shelf/boom.md")]))
            .unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        send.send(RunTrigger::Files(vec![created("shelf/a.md")]))
            .unwrap();
        wait_for_runs(&fx.svc, 2).await;
        assert_eq!(
            seen(&fx.svc).await.as_array().unwrap()[1],
            json!([2, "start", "", "{\"n\": 1}"])
        );
        // A good run clears it: files are files again.
        send.send(RunTrigger::Files(vec![created("shelf/b.md")]))
            .unwrap();
        wait_for_runs(&fx.svc, 3).await;
        assert_eq!(
            seen(&fx.svc).await.as_array().unwrap()[2],
            json!([3, "files", "shelf/b.md:created", "{\"n\": 2}"])
        );
        task.abort();
    }

    async fn run_command(svc: &Arc<Services>, command: &str) {
        svc.commands
            .run(
                &oxplow_domain::Actor::Human,
                command,
                json!({ "name": "shelf" }),
                true,
            )
            .await
            .unwrap();
    }

    async fn wait_for_runs(svc: &Services, n: usize) {
        for _ in 0..200 {
            if seen(svc)
                .await
                .as_array()
                .is_some_and(|rows| rows.len() >= n)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the collector didn't run {n} times: {}", seen(svc).await);
    }

    /// Observers start when their extension is on (the `start` run happens),
    /// stop when it is turned off, and start over when it is turned back on.
    #[tokio::test]
    async fn start_runs_on_boot_and_on_enable_and_never_while_off() {
        let fx = crate::test_fixtures::services_with_thread().await;
        declare(&fx.svc).await;
        let mut running = BTreeMap::new();
        reconcile(&fx.svc, &mut running, observers(&fx.svc));
        assert_eq!(running.len(), 1);
        wait_for_runs(&fx.svc, 1).await;

        // Reconciling again changes nothing: it is not a second start.
        reconcile(&fx.svc, &mut running, observers(&fx.svc));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(seen(&fx.svc).await.as_array().unwrap().len(), 1);

        // Off: no observer, and a trigger already on its way is dropped.
        let o = observer(&fx.svc);
        run_command(&fx.svc, "oxplow.extension.disable").await;
        assert!(observers(&fx.svc).is_empty());
        reconcile(&fx.svc, &mut running, observers(&fx.svc));
        assert!(running.is_empty());
        fire(&fx.svc, &o, RunTrigger::Sqlite).await;
        assert_eq!(seen(&fx.svc).await.as_array().unwrap().len(), 1);

        // On again: it starts over, and its cursor carries on.
        run_command(&fx.svc, "oxplow.extension.enable").await;
        reconcile(&fx.svc, &mut running, observers(&fx.svc));
        wait_for_runs(&fx.svc, 2).await;
        assert_eq!(
            seen(&fx.svc).await.as_array().unwrap()[1],
            json!([2, "start", "", "{\"n\": 1}"])
        );
    }
}
