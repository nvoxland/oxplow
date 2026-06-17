//! Filesystem watcher.
//!
//! Wraps `notify::RecommendedWatcher` and exposes a single broadcast
//! channel of `WatchEvent`s. Raw events fire **immediately** — there is
//! no built-in debounce. Reused by oxplow-git for `.git/refs` watching,
//! and by the snapshot-capture and analysis pipelines.
//!
//! Two ways to consume:
//!   - [`FsWatcher::subscribe`] — the raw, immediate stream. Every
//!     OS-level event flows through with no coalescing. The snapshot
//!     singleton uses this so its dirty set is always current the
//!     instant a snapshot is requested.
//!   - [`FsWatcher::subscribe_debounced`] — a coalescing listener that
//!     batches a burst into at most one event per `(path, kind)` pair
//!     per debounce window. This is what most of the system (file
//!     tree, editor prompts, git-context UI) should watch, so a single
//!     `git checkout` or save-storm doesn't spam every subscriber.
//!
//! Replaces the `chokidar` usage scattered through the TS codebase.
//! The watcher is cancelled by dropping the `FsWatcher` handle —
//! internal threads exit cleanly, no zombie threads.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub use notify::RecursiveMode;
use notify::{RecommendedWatcher, Watcher};
use thiserror::Error;
use tokio::sync::broadcast;

#[derive(Debug, Error)]
pub enum FsWatchError {
    #[error("notify error: {0}")]
    Notify(#[from] notify::Error),
}

/// What happened to a watched path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WatchEventKind {
    Created,
    Modified,
    Removed,
    /// `notify` couldn't classify the event but the path is implicated.
    Other,
}

#[derive(Debug, Clone)]
pub struct WatchEvent {
    pub path: PathBuf,
    pub kind: WatchEventKind,
}

/// Active filesystem watcher.
///
/// Hold this for as long as you want to watch. Drop it to cancel.
pub struct FsWatcher {
    // Holding the watcher alive keeps it running. Drop releases all OS
    // handles.
    _watcher: RecommendedWatcher,
    sender: broadcast::Sender<WatchEvent>,
}

impl FsWatcher {
    /// Watch `path` recursively. Events fire immediately, with no
    /// coalescing — see [`FsWatcher::subscribe_debounced`] for the
    /// batched view.
    pub fn watch(path: impl AsRef<Path>) -> Result<Self, FsWatchError> {
        Self::watch_paths(vec![(
            path.as_ref().to_path_buf(),
            RecursiveMode::Recursive,
        )])
    }

    /// Watch a set of paths with per-path recursion modes. A single
    /// OS-level watcher is shared across every entry, so an event on
    /// any registered path flows through the same broadcast channel.
    /// Events are emitted immediately as the OS reports them.
    pub fn watch_paths(paths: Vec<(PathBuf, RecursiveMode)>) -> Result<Self, FsWatchError> {
        // Capacity is generous because the raw stream is un-coalesced:
        // a `git checkout` or build can fire hundreds of events before
        // a slow subscriber drains them. Lagged subscribers log and
        // recover; they never block the watcher thread.
        let (tx, _) = broadcast::channel(1024);
        let tx_clone = tx.clone();

        let mut watcher =
            notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                let Ok(event) = res else { return };
                let kind = classify(&event);
                for path in event.paths.iter() {
                    let _ = tx_clone.send(WatchEvent {
                        path: path.clone(),
                        kind: kind.clone(),
                    });
                }
            })?;

        for (p, mode) in paths {
            watcher.watch(&p, mode)?;
        }

        Ok(Self {
            _watcher: watcher,
            sender: tx,
        })
    }

    /// Subscribe to the raw, immediate event stream. Every OS-level
    /// event flows through with no coalescing. Subscribers can connect
    /// at any time; events emitted before subscription are dropped.
    pub fn subscribe(&self) -> broadcast::Receiver<WatchEvent> {
        self.sender.subscribe()
    }

    /// Subscribe to a debounced view of the event stream. A burst of
    /// events is collected for `window` after the first one arrives,
    /// then flushed as at most one `WatchEvent` per `(path, kind)`
    /// pair. This is the feed most of the system should watch — it
    /// turns a save-storm or `git checkout` into a single round of
    /// notifications instead of one per touched file.
    ///
    /// The coalescing task lives until the underlying watcher is
    /// dropped (which closes the raw stream); the returned receiver
    /// then closes too.
    pub fn subscribe_debounced(&self, window: Duration) -> broadcast::Receiver<WatchEvent> {
        let mut raw = self.subscribe();
        let (tx, rx) = broadcast::channel(1024);
        tokio::spawn(async move {
            use broadcast::error::RecvError;
            loop {
                // Block until the first event of a new burst (or the
                // raw stream closes).
                let mut pending: HashSet<(PathBuf, WatchEventKind)> = HashSet::new();
                match raw.recv().await {
                    Ok(ev) => {
                        pending.insert((ev.path, ev.kind));
                    }
                    Err(RecvError::Lagged(_)) => continue,
                    Err(RecvError::Closed) => break,
                }
                // Accumulate everything that lands within `window`,
                // then flush the de-duplicated set.
                let deadline = tokio::time::sleep(window);
                tokio::pin!(deadline);
                let mut closed = false;
                loop {
                    tokio::select! {
                        _ = &mut deadline => break,
                        ev = raw.recv() => match ev {
                            Ok(ev) => { pending.insert((ev.path, ev.kind)); }
                            Err(RecvError::Lagged(_)) => {}
                            Err(RecvError::Closed) => { closed = true; break; }
                        },
                    }
                }
                for (path, kind) in pending.drain() {
                    let _ = tx.send(WatchEvent { path, kind });
                }
                if closed {
                    break;
                }
            }
        });
        rx
    }
}

fn classify(event: &notify::Event) -> WatchEventKind {
    use notify::EventKind;
    match event.kind {
        EventKind::Create(_) => WatchEventKind::Created,
        EventKind::Modify(_) => WatchEventKind::Modified,
        EventKind::Remove(_) => WatchEventKind::Removed,
        _ => WatchEventKind::Other,
    }
}

/// Path segments that ALWAYS trigger a watch-ignore, regardless of
/// user config. Limited to `.git` — git's internal write spool
/// churns faster than we can capture and produces ephemeral
/// lock/tmp files that always race the watcher to a NotFound, so
/// watching it is never useful. Everything else (build outputs,
/// language caches, IDE state) is the user's call via the
/// `generated` config; we don't pretend to know which dirs each
/// project actually treats as generated. `.oxplow` is handled
/// separately above with a `.oxplow/wiki/` carve-out, not via this
/// segment list.
const DEFAULT_IGNORED_SEGMENTS: &[&str] = &[".git"];

/// The single source of truth for "is this workspace path ignored?".
/// Built once per project ([`WorkspaceFilter::for_project`]) and shared
/// by cheap clone across snapshot capture, code-quality scans, fs-watch
/// consumers, and the indexer — no consumer keeps its own skip-list.
///
/// A path is ignored when, in precedence order:
/// 1. it is under `.git` or `.oxplow` (except `.oxplow/wiki/`) —
///    absolute, never overridable;
/// 2. otherwise, if it matches an **`include`** entry it is **kept**
///    (force-track something `.gitignore` would otherwise drop);
/// 3. otherwise, if it matches an **`exclude`** entry it is ignored;
/// 4. otherwise, if `.gitignore` (the repo root `.gitignore` +
///    `.git/info/exclude`) ignores it, it is ignored;
/// 5. otherwise it is kept.
///
/// Root-level `.gitignore` patterns are matched at every depth (git
/// semantics), so `node_modules` / `target` / build caches are pruned
/// throughout the tree, and per-subdirectory `.gitignore` files are
/// loaded too — each matched relative to its own directory, so nested
/// patterns (and `!negations` that re-include) work with full git
/// semantics.
///
/// `exclude`/`include` entries are either a single segment (matches any
/// path component — `target` matches every `target/`) or a repo-relative
/// path (matches that path exactly or as a prefix — `apps/desktop/dist`).
#[derive(Clone, Default)]
pub struct WorkspaceFilter {
    exclude: Vec<FilterEntry>,
    include: Vec<FilterEntry>,
    /// Repo root (absolute). `ignore()` joins the repo-relative query
    /// path onto this so it can be matched against each per-directory
    /// matcher, which are rooted at absolute directories.
    root: PathBuf,
    /// One matcher per directory that has a `.gitignore` (the root entry
    /// also folds in `.git/info/exclude`), in shallowest-first order.
    /// Each is rooted at its own directory so nested patterns anchor
    /// correctly; deeper matchers override shallower ones.
    gitignores: std::sync::Arc<Vec<ignore::gitignore::Gitignore>>,
}

impl std::fmt::Debug for WorkspaceFilter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceFilter")
            .field("exclude", &self.exclude)
            .field("include", &self.include)
            .field("gitignores", &self.gitignores.len())
            .finish()
    }
}

#[derive(Debug, Clone)]
enum FilterEntry {
    Segment(String),
    Path(PathBuf),
}

impl WorkspaceFilter {
    /// Exclude-only filter with no `.gitignore` awareness — for tests
    /// and callers that have no project root. Equivalent to
    /// `for_project` with an empty include list and no repo.
    pub fn with_user_entries<I, S>(entries: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self {
            exclude: parse_filter_entries(entries),
            include: Vec::new(),
            root: PathBuf::new(),
            gitignores: std::sync::Arc::new(Vec::new()),
        }
    }

    /// Build the project filter: `.gitignore`-aware (root + nested
    /// `.gitignore` files + `.git/info/exclude`), plus the project's
    /// `generated.exclude` and `generated.include` lists. `root` is the
    /// worktree directory (absolute).
    pub fn for_project<E, S, I, T>(root: &Path, exclude: E, include: I) -> Self
    where
        E: IntoIterator<Item = S>,
        S: AsRef<str>,
        I: IntoIterator<Item = T>,
        T: AsRef<str>,
    {
        Self {
            exclude: parse_filter_entries(exclude),
            include: parse_filter_entries(include),
            root: root.to_path_buf(),
            gitignores: std::sync::Arc::new(collect_gitignores(root)),
        }
    }

    /// True if `path` (workspace-relative) should be ignored. `is_dir`
    /// lets `.gitignore` directory-only patterns (`build/`) prune a
    /// whole subtree without descending into it — pass the real value
    /// at walk sites; `false` is a safe default for file events.
    pub fn ignore(&self, path: &Path, is_dir: bool) -> bool {
        use std::path::Component;

        // 1. Absolute defaults: `.git` anywhere, `.oxplow` (except
        //    `.oxplow/wiki/`). Not overridable by include/exclude.
        let mut comps = path.components().peekable();
        while let Some(c) = comps.next() {
            if let Component::Normal(seg) = c {
                let s = match seg.to_str() {
                    Some(s) => s,
                    None => continue,
                };
                if s == ".oxplow" {
                    let next = comps.peek().and_then(|c| match c {
                        Component::Normal(n) => n.to_str(),
                        _ => None,
                    });
                    return next != Some("wiki");
                }
                if DEFAULT_IGNORED_SEGMENTS.contains(&s) {
                    return true;
                }
            }
        }

        // 2. `include` forces a path back in (overrides exclude + gitignore).
        if matches_filter_entries(&self.include, path) {
            return false;
        }
        // 3. Explicit excludes.
        if matches_filter_entries(&self.exclude, path) {
            return true;
        }
        // 4. `.gitignore` — root + nested, full hierarchical semantics.
        if !self.gitignores.is_empty() && !path.as_os_str().is_empty() {
            let abs = self.root.join(path);
            if gitignore_decision(&self.gitignores, &abs, is_dir).is_ignore() {
                return true;
            }
        }
        false
    }
}

fn parse_filter_entries<I, S>(entries: I) -> Vec<FilterEntry>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    entries
        .into_iter()
        .filter_map(|raw| {
            let trimmed = raw.as_ref().trim().trim_matches('/');
            if trimmed.is_empty() {
                None
            } else if trimmed.contains('/') {
                Some(FilterEntry::Path(PathBuf::from(trimmed)))
            } else {
                Some(FilterEntry::Segment(trimmed.to_string()))
            }
        })
        .collect()
}

fn matches_filter_entries(entries: &[FilterEntry], path: &Path) -> bool {
    use std::path::Component;
    for entry in entries {
        match entry {
            FilterEntry::Segment(seg) => {
                if path
                    .components()
                    .any(|c| matches!(c, Component::Normal(n) if n.to_str() == Some(seg.as_str())))
                {
                    return true;
                }
            }
            FilterEntry::Path(p) => {
                if path == p.as_path() || path.starts_with(p) {
                    return true;
                }
            }
        }
    }
    false
}

/// Build the per-directory `.gitignore` matcher set for `root`,
/// shallowest-first: the root matcher (`.git/info/exclude` + root
/// `.gitignore`) followed by every nested `.gitignore` reachable without
/// descending into an already-ignored directory. Each matcher is rooted
/// at its own directory so nested patterns anchor correctly.
fn collect_gitignores(root: &Path) -> Vec<ignore::gitignore::Gitignore> {
    use ignore::gitignore::GitignoreBuilder;
    let mut out = Vec::new();
    // `.git/info/exclude` first (lower precedence), then root `.gitignore`.
    let mut builder = GitignoreBuilder::new(root);
    let _ = builder.add(root.join(".git").join("info").join("exclude"));
    let _ = builder.add(root.join(".gitignore"));
    if let Ok(gi) = builder.build() {
        out.push(gi);
    }
    collect_nested_gitignores(root, &mut out);
    out
}

/// Recurse `dir`'s subdirectories, pruning any the matchers-so-far
/// already ignore (so we never descend into `node_modules`/`target`/…),
/// and append each surviving subdirectory's `.gitignore` matcher.
fn collect_nested_gitignores(dir: &Path, out: &mut Vec<ignore::gitignore::Gitignore>) {
    use ignore::gitignore::GitignoreBuilder;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let abs = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == ".git" || name == ".oxplow" {
            continue;
        }
        // Don't descend into (or load `.gitignore`s from) an ignored dir.
        if gitignore_decision(out, &abs, true).is_ignore() {
            continue;
        }
        let gi_path = abs.join(".gitignore");
        if gi_path.is_file() {
            let mut builder = GitignoreBuilder::new(&abs);
            let _ = builder.add(&gi_path);
            if let Ok(gi) = builder.build() {
                out.push(gi);
            }
        }
        collect_nested_gitignores(&abs, out);
    }
}

/// Hierarchical match of the absolute path `abs` against the directory
/// matcher set. A matcher applies only when `abs` is under its directory;
/// deeper matchers (later in the list) override shallower ones, so a
/// nested `!negation` re-includes a path a parent `.gitignore` dropped.
fn gitignore_decision(
    gitignores: &[ignore::gitignore::Gitignore],
    abs: &Path,
    is_dir: bool,
) -> ignore::Match<()> {
    let mut decision = ignore::Match::None;
    for gi in gitignores {
        if abs.starts_with(gi.path()) {
            match gi.matched_path_or_any_parents(abs, is_dir) {
                ignore::Match::None => {}
                ignore::Match::Ignore(_) => decision = ignore::Match::Ignore(()),
                ignore::Match::Whitelist(_) => decision = ignore::Match::Whitelist(()),
            }
        }
    }
    decision
}

/// Default-only shorthand for callers that don't have a configured
/// filter handy (e.g. the snapshot-sweep example binary). Applies the
/// always-on `.git` / `.oxplow` defaults only — no gitignore, no user
/// entries.
pub fn should_ignore_workspace_watch_path(path: &Path) -> bool {
    WorkspaceFilter::default().ignore(path, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::tempdir;
    use tokio::time::timeout;

    #[tokio::test]
    async fn detects_new_file() {
        let dir = tempdir().unwrap();
        let watcher = FsWatcher::watch(dir.path()).unwrap();
        let mut rx = watcher.subscribe();

        let target = dir.path().join("hello.txt");
        std::fs::write(&target, b"hello").unwrap();

        let evt = timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("event")
            .expect("event ok");
        // macOS resolves /var/folders to /private/var/folders — compare
        // canonicalized paths so the test is stable across platforms.
        assert_eq!(
            evt.path.canonicalize().unwrap(),
            target.canonicalize().unwrap()
        );
    }

    #[tokio::test]
    async fn debounced_subscription_coalesces_rapid_writes() {
        let dir = tempdir().unwrap();
        let watcher = FsWatcher::watch(dir.path()).unwrap();
        let mut rx = watcher.subscribe_debounced(Duration::from_millis(150));

        let target = dir.path().join("hot.txt");
        for i in 0..20 {
            std::fs::write(&target, format!("{i}")).unwrap();
        }

        // Collect everything that lands within 1s. The debounced view
        // should coalesce 20 writes into a small number of events.
        let mut count = 0;
        while let Ok(Ok(_)) = timeout(Duration::from_millis(800), rx.recv()).await {
            count += 1;
        }

        assert!(count > 0, "expected at least one event");
        assert!(count < 20, "expected debouncing, got {count} events");
    }

    #[tokio::test]
    async fn raw_subscription_fires_per_distinct_file() {
        // The raw stream does not coalesce across distinct paths: a
        // write to each of N files surfaces N separate events. (Same
        // file may dedup at the OS layer, so we use distinct files.)
        let dir = tempdir().unwrap();
        let watcher = FsWatcher::watch(dir.path()).unwrap();
        let mut rx = watcher.subscribe();

        let mut targets = HashSet::new();
        for i in 0..5 {
            let p = dir.path().join(format!("f{i}.txt"));
            std::fs::write(&p, b"x").unwrap();
            targets.insert(p.canonicalize().unwrap());
        }

        let mut seen = HashSet::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline && seen.len() < targets.len() {
            if let Ok(Ok(evt)) = timeout(Duration::from_millis(500), rx.recv()).await {
                if let Ok(canon) = evt.path.canonicalize() {
                    if targets.contains(&canon) {
                        seen.insert(canon);
                    }
                }
            }
        }
        assert_eq!(seen, targets, "raw stream should surface each file");
    }

    #[tokio::test]
    async fn watch_paths_registers_multiple_dirs() {
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        let watcher = FsWatcher::watch_paths(vec![
            (a.path().to_path_buf(), RecursiveMode::Recursive),
            (b.path().to_path_buf(), RecursiveMode::Recursive),
        ])
        .unwrap();
        let mut rx = watcher.subscribe();

        let ta = a.path().join("a.txt");
        let tb = b.path().join("b.txt");
        std::fs::write(&ta, b"a").unwrap();
        std::fs::write(&tb, b"b").unwrap();

        let mut saw_a = false;
        let mut saw_b = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline && !(saw_a && saw_b) {
            if let Ok(Ok(evt)) = timeout(Duration::from_millis(500), rx.recv()).await {
                let p = evt.path.canonicalize().unwrap_or(evt.path.clone());
                if p == ta.canonicalize().unwrap() {
                    saw_a = true;
                }
                if p == tb.canonicalize().unwrap() {
                    saw_b = true;
                }
            }
        }
        assert!(saw_a, "expected event for {ta:?}");
        assert!(saw_b, "expected event for {tb:?}");
    }

    #[tokio::test]
    async fn non_recursive_only_reports_top_level() {
        let dir = tempdir().unwrap();
        let sub = dir.path().join("nested");
        std::fs::create_dir(&sub).unwrap();

        let watcher = FsWatcher::watch_paths(vec![(
            dir.path().to_path_buf(),
            RecursiveMode::NonRecursive,
        )])
        .unwrap();
        let mut rx = watcher.subscribe();

        // Write into a subdir — should NOT show up under non-recursive.
        let nested = sub.join("hidden.txt");
        std::fs::write(&nested, b"x").unwrap();

        // Drain for ~600ms; if we see the nested path, fail.
        let drain_deadline = std::time::Instant::now() + Duration::from_millis(600);
        let mut nested_seen = false;
        while std::time::Instant::now() < drain_deadline {
            match timeout(Duration::from_millis(150), rx.recv()).await {
                Ok(Ok(evt)) => {
                    let canon = evt.path.canonicalize().unwrap_or(evt.path.clone());
                    if canon == nested.canonicalize().unwrap() {
                        nested_seen = true;
                        break;
                    }
                }
                _ => break,
            }
        }
        assert!(
            !nested_seen,
            "non-recursive watch should not surface nested writes"
        );

        // Top-level write should be reported.
        let top = dir.path().join("top.txt");
        std::fs::write(&top, b"y").unwrap();
        let mut top_seen = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            match timeout(Duration::from_millis(500), rx.recv()).await {
                Ok(Ok(evt)) => {
                    let canon = evt.path.canonicalize().unwrap_or(evt.path.clone());
                    if canon == top.canonicalize().unwrap() {
                        top_seen = true;
                        break;
                    }
                }
                _ => break,
            }
        }
        assert!(top_seen, "expected top-level event for {top:?}");
    }

    #[test]
    fn should_ignore_filters_oxplow_and_git() {
        // Always-on: anything under `.oxplow/` except `.oxplow/wiki/`,
        // and anything under `.git/`. Everything else is the user's
        // call via the `generated` config.
        assert!(should_ignore_workspace_watch_path(Path::new(
            ".oxplow/snapshots/aa/foo.tmp"
        )));
        assert!(!should_ignore_workspace_watch_path(Path::new(
            ".oxplow/wiki/local-snapshots.md"
        )));
        assert!(should_ignore_workspace_watch_path(Path::new(
            ".oxplow/state.sqlite"
        )));
        assert!(should_ignore_workspace_watch_path(Path::new(
            ".git/index.lock"
        )));
        // Build dirs are NOT default-ignored; users opt in via `generated`.
        assert!(!should_ignore_workspace_watch_path(Path::new(
            "target/debug/x.bin"
        )));
        assert!(!should_ignore_workspace_watch_path(Path::new(
            "node_modules/foo/index.js"
        )));
        assert!(!should_ignore_workspace_watch_path(Path::new(
            "src/main.rs"
        )));
        assert!(!should_ignore_workspace_watch_path(Path::new(
            "docs/README.md"
        )));
    }

    #[test]
    fn workspace_filter_user_segment_matches_anywhere() {
        let f = WorkspaceFilter::with_user_entries([".idea"]);
        assert!(f.ignore(Path::new(".idea/workspace.xml"), false));
        assert!(f.ignore(Path::new("nested/.idea/foo"), false));
        assert!(!f.ignore(Path::new("src/main.rs"), false));
    }

    #[test]
    fn workspace_filter_user_path_matches_prefix_only() {
        // Use a path entry whose segments don't collide with the
        // always-on defaults (no `dist`, `target`, etc.). That lets
        // us isolate the "matches prefix only" semantics.
        let f = WorkspaceFilter::with_user_entries(["apps/desktop/generated"]);
        assert!(f.ignore(Path::new("apps/desktop/generated"), false));
        assert!(f.ignore(Path::new("apps/desktop/generated/index.js"), false));
        assert!(!f.ignore(Path::new("apps/desktop"), false));
        // Not a free-floating match — only the exact prefix counts.
        assert!(!f.ignore(Path::new("crates/foo/apps/desktop/generated/x"), false));
    }

    #[test]
    fn workspace_filter_user_file_path_matches_exact() {
        let f = WorkspaceFilter::with_user_entries(["docs/generated/output.txt"]);
        assert!(f.ignore(Path::new("docs/generated/output.txt"), false));
        assert!(!f.ignore(Path::new("docs/generated/other.txt"), false));
    }

    #[test]
    fn workspace_filter_defaults_apply_even_with_empty_user_list() {
        // Defaults are `.git` (segment) and `.oxplow/*` (with a
        // `.oxplow/wiki/` carve-out). Build dirs are NOT defaults —
        // they require the user to add them to `generated`.
        let f = WorkspaceFilter::default();
        assert!(f.ignore(Path::new(".git/HEAD"), false));
        assert!(f.ignore(Path::new("crates/foo/.git/HEAD"), false));
        assert!(!f.ignore(Path::new("node_modules/x"), false));
        assert!(!f.ignore(Path::new("target/debug"), false));
        assert!(!f.ignore(Path::new("src/main.rs"), false));
    }

    #[test]
    fn workspace_filter_oxplow_wiki_passes_through() {
        let f = WorkspaceFilter::default();
        assert!(!f.ignore(Path::new(".oxplow/wiki/page.md"), false));
        assert!(f.ignore(Path::new(".oxplow/state.sqlite"), false));
    }

    #[test]
    fn workspace_filter_empty_entries_are_dropped() {
        let f = WorkspaceFilter::with_user_entries(["", "  ", "/"]);
        assert!(!f.ignore(Path::new("foo.txt"), false));
    }

    fn empty() -> Vec<String> {
        Vec::new()
    }

    #[test]
    fn for_project_respects_root_gitignore_at_every_depth() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join(".gitignore"),
            "node_modules/\n*.log\ndist/\n",
        )
        .unwrap();
        let f = WorkspaceFilter::for_project(dir.path(), empty(), empty());
        // Root-level pattern, matched at every depth (git semantics).
        assert!(f.ignore(Path::new("node_modules"), true));
        assert!(f.ignore(Path::new("node_modules/pkg/index.js"), false));
        assert!(f.ignore(Path::new("crates/foo/node_modules/x"), false));
        assert!(f.ignore(Path::new("build.log"), false));
        assert!(f.ignore(Path::new("dist/app.js"), false));
        // Tracked sources pass through.
        assert!(!f.ignore(Path::new("src/main.rs"), false));
        assert!(!f.ignore(Path::new("README.md"), false));
    }

    #[test]
    fn for_project_include_overrides_gitignore() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "dist/\n").unwrap();
        let f = WorkspaceFilter::for_project(dir.path(), empty(), ["dist/keep.json"]);
        assert!(f.ignore(Path::new("dist/other.js"), false)); // still ignored
        assert!(!f.ignore(Path::new("dist/keep.json"), false)); // forced back in
    }

    #[test]
    fn for_project_exclude_adds_beyond_gitignore() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "node_modules/\n").unwrap();
        // A noisy dir that isn't in .gitignore but we still don't want.
        let f = WorkspaceFilter::for_project(dir.path(), ["snapshots-out"], empty());
        assert!(f.ignore(Path::new("snapshots-out/big.json"), false));
        assert!(f.ignore(Path::new("node_modules/x"), false));
        assert!(!f.ignore(Path::new("src/main.rs"), false));
    }

    #[test]
    fn for_project_dir_only_pattern_uses_is_dir() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "build/\n").unwrap();
        let f = WorkspaceFilter::for_project(dir.path(), empty(), empty());
        // `build/` is dir-only: a directory named `build` is pruned...
        assert!(f.ignore(Path::new("build"), true));
        // ...but a *file* literally named `build` is not matched by `build/`.
        assert!(!f.ignore(Path::new("build"), false));
    }

    #[test]
    fn for_project_without_gitignore_is_defaults_only() {
        let dir = tempdir().unwrap(); // no .gitignore written
        let f = WorkspaceFilter::for_project(dir.path(), empty(), empty());
        assert!(f.ignore(Path::new(".git/HEAD"), false));
        assert!(!f.ignore(Path::new("node_modules/x"), false));
        assert!(!f.ignore(Path::new("src/main.rs"), false));
    }

    #[test]
    fn for_project_loads_nested_gitignore_additions() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "*.log\n").unwrap();
        std::fs::create_dir_all(dir.path().join("sub/local-out")).unwrap();
        // A nested .gitignore adds a pattern meaningful only under sub/.
        std::fs::write(dir.path().join("sub/.gitignore"), "local-out/\n").unwrap();
        let f = WorkspaceFilter::for_project(dir.path(), empty(), empty());
        // Root pattern still applies at every depth.
        assert!(f.ignore(Path::new("a.log"), false));
        assert!(f.ignore(Path::new("sub/b.log"), false));
        // Nested pattern applies under sub/ ...
        assert!(f.ignore(Path::new("sub/local-out"), true));
        assert!(f.ignore(Path::new("sub/local-out/x.txt"), false));
        // ... but NOT at the root — it's a nested addition, not global.
        assert!(!f.ignore(Path::new("local-out"), true));
        assert!(!f.ignore(Path::new("src/main.rs"), false));
    }

    #[test]
    fn for_project_nested_negation_reincludes() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "*.secret\n").unwrap();
        std::fs::create_dir_all(dir.path().join("keep")).unwrap();
        // A deeper .gitignore re-includes via `!` — git's negation semantics.
        std::fs::write(dir.path().join("keep/.gitignore"), "!*.secret\n").unwrap();
        let f = WorkspaceFilter::for_project(dir.path(), empty(), empty());
        assert!(f.ignore(Path::new("a.secret"), false)); // ignored at root
        assert!(!f.ignore(Path::new("keep/a.secret"), false)); // re-included under keep/
    }

    #[tokio::test]
    async fn drop_cancels_watcher() {
        let dir = tempdir().unwrap();
        let watcher = FsWatcher::watch(dir.path()).unwrap();
        let mut rx = watcher.subscribe();
        drop(watcher);
        // After drop, no further events are emitted; channel closes
        // when the sender is dropped (the watcher held the only sender).
        let target = dir.path().join("ignored.txt");
        std::fs::write(&target, b"x").unwrap();
        let recv = timeout(Duration::from_millis(500), rx.recv()).await;
        // Either a timeout (the channel is silent) or a closed channel.
        match recv {
            Err(_) => {}                                       // timeout — fine
            Ok(Err(broadcast::error::RecvError::Closed)) => {} // closed — fine
            other => panic!("expected closed/timeout, got {other:?}"),
        }
    }
}
