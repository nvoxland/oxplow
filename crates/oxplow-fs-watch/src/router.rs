//! What happens to an OS event: [`Router::handle`] runs on the OS callback
//! thread and decides what reaches the subscribers, and [`Follower`] moves the
//! watches of a workspace watch where the OS needs one per directory. Both are
//! plain code over plain values, so they are tested by handing them made-up
//! events (`tests` below), with no OS.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, Weak};

use notify::RecursiveMode;

use crate::filter::{is_ignore_file, WorkspaceFilter};
use crate::pending::Subscribers;
use crate::registrar::Registrar;
use crate::WatchEventKind;

/// How a workspace watch asks the OS to watch a tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Registration {
    /// One recursive watch on the root: the OS reports everything under it,
    /// for one registration, and the router drops what the filter ignores
    /// (FSEvents, `ReadDirectoryChangesW`). A folder that appears is covered
    /// at once.
    Root,
    /// One watch per directory the filter keeps, at any depth (inotify, which
    /// has no recursive watch and a limit on how many it holds): an ignored
    /// folder costs nothing, and [`Follower`] covers a kept one that appears.
    PerDirectory,
}

impl Registration {
    pub(crate) fn for_this_os() -> Self {
        if cfg!(target_os = "linux") {
            Self::PerDirectory
        } else {
            Self::Root
        }
    }
}

/// What the router tells the follower.
#[derive(Debug)]
pub(crate) enum FollowMsg {
    /// Something appeared at this path, or moved here: perhaps a directory.
    Appeared(PathBuf),
    /// This path went.
    Gone(PathBuf),
    /// The rules changed for what is under this directory.
    Refilter(PathBuf),
}

/// A workspace watch's half of the router.
pub(crate) struct Route {
    /// The root, as given and canonical: the OS may report either spelling
    /// (macOS reports `/private/var` for `/var`).
    pub(crate) roots: Vec<PathBuf>,
    pub(crate) filter: WorkspaceFilter,
    /// Off the OS callback thread: registering a watch from it can deadlock
    /// a backend. Only a [`Registration::PerDirectory`] watch has one.
    pub(crate) follower: Option<mpsc::Sender<FollowMsg>>,
}

/// Decides what an OS event means.
pub(crate) struct Router {
    pub(crate) events: Subscribers,
    pub(crate) route: Option<Route>,
}

impl Router {
    pub(crate) fn handle(&self, res: notify::Result<notify::Event>) {
        let event = match res {
            Ok(event) => event,
            Err(e) => {
                tracing::warn!(error = %e, "file watch error: checking the disk");
                self.lost(None);
                return;
            }
        };
        // A read changes nothing. inotify reports one (open, close
        // after reading) for a file the watcher's own consumer is
        // reading, and a consumer that reads on every event —
        // the config reload — would otherwise hear itself: an
        // endless loop that kept a daemon's boot from finishing.
        if matches!(event.kind, notify::EventKind::Access(_)) {
            return;
        }
        if event.need_rescan() {
            self.lost(event.paths.first().map(PathBuf::as_path));
            return;
        }
        let kind = classify(&event);
        for path in &event.paths {
            match &self.route {
                Some(route) => self.route_change(route, path, &kind, &event.kind),
                None => self.events.change(path, &kind, true),
            }
        }
    }

    fn route_change(
        &self,
        route: &Route,
        path: &Path,
        kind: &WatchEventKind,
        os_kind: &notify::EventKind,
    ) {
        let rel = route.relative(path);
        if is_ignore_file(path) && !route.in_ignored_dir(path) {
            // What is written next is judged by the new rules. What they
            // stopped ignoring, no one has heard of: check the disk under the
            // directory they rule, and move the watches.
            route.filter.note_ignore_file(rel.unwrap_or(path));
            let dir = route.ruled_by(path);
            self.events.rescan(&dir);
            route.tell(FollowMsg::Refilter(dir));
        } else if rel.is_some_and(|rel| route.filter.ignore_change(rel, || path.is_dir())) {
            // A build writing thousands of files under an ignored folder
            // (inside a kept directory, where the OS watch still sees it) must
            // cost next to nothing. Only a scope that names the path hears it.
            self.events.change(path, kind, false);
            return;
        }
        // In the root's own spelling, whichever the OS used: a subscriber
        // matches what it hears against the root it knows.
        let spelled = rel.and_then(|rel| route.roots.first().map(|root| root.join(rel)));
        let path = spelled.as_deref().unwrap_or(path);
        self.events.change(path, kind, true);
        match os_kind {
            notify::EventKind::Create(_)
            | notify::EventKind::Modify(notify::event::ModifyKind::Name(_)) => {
                route.tell(FollowMsg::Appeared(path.to_path_buf()));
            }
            notify::EventKind::Remove(_) => route.tell(FollowMsg::Gone(path.to_path_buf())),
            _ => {}
        }
    }

    /// The OS lost changes (under `path`, when it says): subscribers check
    /// the disk. Loss under an ignored folder (a build overflowing the OS's
    /// queue) is no one's concern.
    fn lost(&self, path: Option<&Path>) {
        if let Some(route) = &self.route {
            route.filter.forget_rules();
        }
        let Some(path) = path else {
            self.events.rescan_all();
            return;
        };
        match &self.route {
            Some(route) => match route.relative(path) {
                Some(rel) if route.filter.ignore_change(rel, || true) => {}
                Some(_) => self.events.rescan(path),
                None => self.events.rescan_all(),
            },
            None => self.events.rescan(path),
        }
    }
}

impl Route {
    /// `path` from the root. A path outside every root is not judged.
    fn relative<'p>(&self, path: &'p Path) -> Option<&'p Path> {
        self.roots.iter().find_map(|r| path.strip_prefix(r).ok())
    }

    fn tell(&self, msg: FollowMsg) {
        if let Some(follower) = &self.follower {
            let _ = follower.send(msg);
        }
    }

    /// Whether an ignore file at `path` sits in an ignored folder: its rules
    /// are no one's (a build tool writes `target/.gitignore`).
    fn in_ignored_dir(&self, path: &Path) -> bool {
        self.relative(path)
            .and_then(Path::parent)
            .is_some_and(|dir| {
                !dir.as_os_str().is_empty()
                    && !dir.ends_with(".git/info")
                    && self.filter.ignore_change(dir, || true)
            })
    }

    /// The directory an ignore file at `path` rules: its own, or the whole
    /// root for an exclude file.
    fn ruled_by(&self, path: &Path) -> PathBuf {
        let root = || self.roots.first().cloned().unwrap_or_default();
        match self.relative(path) {
            Some(rel) if !rel.ends_with(".git/info/exclude") => {
                path.parent().map_or_else(root, Path::to_path_buf)
            }
            _ => root(),
        }
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

/// Directories the watch could not cover (the OS's limit), which only a look
/// at the disk can speak for. One is listed for all that is under it.
#[derive(Clone, Default)]
pub struct Uncovered(Arc<Mutex<BTreeSet<PathBuf>>>);

impl Uncovered {
    pub fn new(dirs: impl IntoIterator<Item = PathBuf>) -> Self {
        Self(Arc::new(Mutex::new(dirs.into_iter().collect())))
    }

    pub fn list(&self) -> Vec<PathBuf> {
        self.lock().iter().cloned().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeSet<PathBuf>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A [`Registration::PerDirectory`] watch's follower: one watch for each
/// directory the filter keeps. One that appears gets its watches and a rescan,
/// since what was written before they landed no watch saw.
pub(crate) struct Follower {
    /// The root.
    pub(crate) root: PathBuf,
    /// Shared with the router.
    pub(crate) filter: WorkspaceFilter,
    /// The directories with a watch.
    pub(crate) watched: HashSet<PathBuf>,
    pub(crate) uncovered: Uncovered,
    pub(crate) events: Subscribers,
}

impl Follower {
    /// Until the watcher is dropped: that drops the router, and with it the
    /// only senders of `messages`.
    pub(crate) fn run<R: Registrar + 'static>(
        mut self,
        watcher: &Weak<Mutex<R>>,
        messages: &mpsc::Receiver<FollowMsg>,
    ) {
        while let Ok(msg) = messages.recv() {
            let Some(watcher) = watcher.upgrade() else {
                return;
            };
            self.handle(msg, &*watcher);
        }
    }

    pub(crate) fn handle(&mut self, msg: FollowMsg, watcher: &Mutex<impl Registrar + ?Sized>) {
        match msg {
            FollowMsg::Appeared(path) => self.appeared(&path, watcher),
            FollowMsg::Gone(path) => self.gone(&path, watcher),
            FollowMsg::Refilter(dir) => self.refilter(&dir, watcher),
        }
    }

    fn ignores(&self, dir: &Path) -> bool {
        dir.strip_prefix(&self.root)
            .is_ok_and(|rel| self.filter.ignore(rel, true))
    }

    /// Watch `dir` and each directory the filter keeps under it. Past the OS's
    /// limit, what is left is listed as uncovered, and the walk stops.
    pub(crate) fn cover(&mut self, dir: &Path, watcher: &Mutex<impl Registrar + ?Sized>) {
        let mut stack = vec![dir.to_path_buf()];
        let mut w = watcher.lock().unwrap_or_else(|e| e.into_inner());
        while let Some(dir) = stack.pop() {
            if !self.watched.contains(&dir) {
                match w.watch(&dir, RecursiveMode::NonRecursive) {
                    Ok(()) => {
                        self.watched.insert(dir.clone());
                    }
                    Err(e) if matches!(e.kind, notify::ErrorKind::MaxFilesWatch) => {
                        tracing::warn!(
                            ?dir,
                            "the OS's limit on file watches is reached: changes here are found \
                             only when a snapshot checks the disk. Raise it with \
                             `sudo sysctl fs.inotify.max_user_watches=<more>`."
                        );
                        self.uncovered.lock().insert(dir);
                        continue;
                    }
                    // Gone already, or unreadable: nothing to watch there.
                    Err(e) => {
                        tracing::debug!(error = %e, ?dir, "could not watch a directory");
                        continue;
                    }
                }
            }
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|t| t.is_dir()) && !self.ignores(&entry.path()) {
                    stack.push(entry.path());
                }
            }
        }
    }

    fn appeared(&mut self, path: &Path, watcher: &Mutex<impl Registrar + ?Sized>) {
        if !path.is_dir() || self.watched.contains(path) || self.ignores(path) {
            return;
        }
        self.cover(path, watcher);
        self.events.rescan(path);
    }

    fn gone(&mut self, path: &Path, watcher: &Mutex<impl Registrar + ?Sized>) {
        self.forget_under(path, watcher);
    }

    /// Stop watching `path` and all under it.
    fn forget_under(&mut self, path: &Path, watcher: &Mutex<impl Registrar + ?Sized>) {
        let under: Vec<PathBuf> = self
            .watched
            .iter()
            .filter(|dir| dir.starts_with(path))
            .cloned()
            .collect();
        let mut w = watcher.lock().unwrap_or_else(|e| e.into_inner());
        for dir in under {
            let _ = w.unwatch(&dir);
            self.watched.remove(&dir);
        }
    }

    /// The rules changed for what is under `dir` (the router has noted them,
    /// and asked for its rescan): stop watching what they now ignore, and watch
    /// what they stopped ignoring.
    fn refilter(&mut self, dir: &Path, watcher: &Mutex<impl Registrar + ?Sized>) {
        let now_ignored: Vec<PathBuf> = self
            .watched
            .iter()
            .filter(|watched| watched.starts_with(dir) && self.ignores(watched))
            .cloned()
            .collect();
        for ignored in now_ignored {
            self.forget_under(&ignored, watcher);
        }
        self.cover(dir, watcher);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pending::Subscription;
    use notify::event::{CreateKind, DataChange, Flag, ModifyKind, RemoveKind};
    use notify::{Event, EventKind};
    use tempfile::{tempdir, TempDir};

    /// Records what the follower registers.
    #[derive(Default)]
    struct FakeRegistrar {
        watched: Vec<PathBuf>,
        unwatched: Vec<PathBuf>,
        /// Refuses a watch past this many, as the OS does at its limit.
        limit: Option<usize>,
    }

    impl Registrar for FakeRegistrar {
        fn watch(&mut self, path: &Path, _: RecursiveMode) -> notify::Result<()> {
            if self.limit.is_some_and(|limit| self.watched.len() >= limit) {
                return Err(notify::Error::new(notify::ErrorKind::MaxFilesWatch));
            }
            self.watched.push(path.to_path_buf());
            Ok(())
        }
        fn unwatch(&mut self, path: &Path) -> notify::Result<()> {
            self.unwatched.push(path.to_path_buf());
            Ok(())
        }
    }

    /// A workspace watch with no OS: an event is handed in, and everything
    /// it sets off has happened when [`Self::event`] returns. With a follower,
    /// it is a [`Registration::PerDirectory`] watch that has covered the tree.
    struct Rig {
        dir: TempDir,
        router: Router,
        follower: Follower,
        messages: mpsc::Receiver<FollowMsg>,
        registrar: Mutex<FakeRegistrar>,
        heard: Subscription,
        rescans: Vec<String>,
    }

    impl Rig {
        fn build(prepare: impl FnOnce(&Path), per_directory: bool, limit: Option<usize>) -> Self {
            let dir = tempdir().unwrap();
            let root = dir.path().to_path_buf();
            prepare(&root);
            let filter =
                WorkspaceFilter::build(&root, Vec::<String>::new(), Vec::<String>::new(), None);
            let events = Subscribers::new(vec![root.clone()], vec![root.clone()]);
            let heard = events.subscribe(None);
            let (follower_tx, messages) = mpsc::channel();
            let mut rig = Self {
                router: Router {
                    events: events.clone(),
                    route: Some(Route {
                        roots: vec![root.clone()],
                        filter: filter.clone(),
                        follower: per_directory.then_some(follower_tx),
                    }),
                },
                follower: Follower {
                    root,
                    filter,
                    watched: HashSet::new(),
                    uncovered: Uncovered::default(),
                    events,
                },
                messages,
                registrar: Mutex::new(FakeRegistrar {
                    limit,
                    ..FakeRegistrar::default()
                }),
                heard,
                rescans: Vec::new(),
                dir,
            };
            if per_directory {
                let root = rig.root().to_path_buf();
                rig.follower.cover(&root, &rig.registrar);
            }
            rig
        }

        /// One watch on the root: the router alone.
        fn root_watch(prepare: impl FnOnce(&Path)) -> Self {
            Self::build(prepare, false, None)
        }

        /// One watch per kept directory.
        fn per_directory(prepare: impl FnOnce(&Path)) -> Self {
            Self::build(prepare, true, None)
        }

        fn root(&self) -> &Path {
            self.dir.path()
        }

        fn event(&mut self, kind: EventKind, rel: &str) {
            let event = Event::new(kind).add_path(self.root().join(rel));
            self.router.handle(Ok(event));
            while let Ok(msg) = self.messages.try_recv() {
                self.follower.handle(msg, &self.registrar);
            }
        }

        /// A modification of `rel`, which the file's own write need not back.
        fn modified(&mut self, rel: &str) {
            self.event(
                EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                rel,
            );
        }

        /// The paths heard since the last call, from the root. Rescans
        /// asked for since then go to [`Self::rescans`].
        fn heard(&mut self) -> Vec<String> {
            let root = self.dir.path().to_path_buf();
            let rel = |p: &Path| {
                p.strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            };
            let batch = self.heard.try_batch().unwrap_or_default();
            self.rescans = batch.rescans.iter().map(|p| rel(p)).collect();
            let mut paths: Vec<String> = batch.paths.iter().map(|(p, _)| rel(p)).collect();
            paths.sort();
            paths
        }

        /// The directories watched, from the root, sorted.
        fn watched(&self) -> Vec<String> {
            let root = self.root();
            let mut dirs: Vec<String> = self
                .registrar
                .lock()
                .unwrap()
                .watched
                .iter()
                .map(|p| p.strip_prefix(root).unwrap().to_string_lossy().into_owned())
                .collect();
            dirs.sort();
            dirs
        }

        fn forget_registered(&self) {
            let mut registrar = self.registrar.lock().unwrap();
            registrar.watched.clear();
            registrar.unwatched.clear();
        }
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// With one watch per directory, an ignored folder gets none, at any
    /// depth: what it costs follows what is kept.
    #[test]
    fn per_directory_watches_only_what_the_filter_keeps_at_any_depth() {
        let rig = Rig::per_directory(|root| {
            write(root, ".gitignore", "node_modules/\ntarget/\n");
            write(root, "src/deep/a.rs", "");
            write(root, "pkg/node_modules/dep/index.js", "");
            write(root, "pkg/lib/b.js", "");
            write(root, "target/debug/x", "");
            write(root, ".git/HEAD", "");
        });
        assert_eq!(rig.watched(), ["", "pkg", "pkg/lib", "src", "src/deep"]);
    }

    /// `mkdir -p d/e && write d/e/f` races the new watches, and no watch sees
    /// the gap: a kept directory that appears gets its watches, down through
    /// what's already in it, and a rescan. What is written after is heard like
    /// anything else.
    #[test]
    fn a_directory_that_appears_is_covered_and_rescanned() {
        let mut rig = Rig::per_directory(|_| {});
        rig.forget_registered();
        write(rig.root(), "new/deep/f.txt", "x");
        rig.event(EventKind::Create(CreateKind::Folder), "new");
        assert_eq!(rig.watched(), ["new", "new/deep"]);
        rig.heard();
        assert_eq!(rig.rescans, ["new"]);

        rig.modified("new/deep/page.yaml");
        assert_eq!(rig.heard(), ["new/deep/page.yaml"]);
    }

    /// A directory moved into place is the same as one made there.
    #[test]
    fn a_directory_moved_here_is_covered_too() {
        let mut rig = Rig::per_directory(|_| {});
        rig.forget_registered();
        write(rig.root(), "moved/f.txt", "x");
        rig.event(
            EventKind::Modify(ModifyKind::Name(notify::event::RenameMode::To)),
            "moved",
        );
        assert_eq!(rig.watched(), ["moved"]);
    }

    /// A directory that goes is forgotten, with all under it, so one of the
    /// same name is new.
    #[test]
    fn a_directory_that_goes_is_new_when_it_comes_back() {
        let mut rig = Rig::per_directory(|root| write(root, "built/deep/f", ""));
        rig.forget_registered();
        std::fs::remove_dir_all(rig.root().join("built")).unwrap();
        rig.event(EventKind::Remove(RemoveKind::Folder), "built");
        assert_eq!(rig.registrar.lock().unwrap().unwatched.len(), 2);

        std::fs::create_dir(rig.root().join("built")).unwrap();
        rig.event(EventKind::Create(CreateKind::Folder), "built");
        assert_eq!(rig.watched(), ["built"]);
    }

    /// A directory the filter drops stays unwatched when it appears.
    #[test]
    fn an_ignored_directory_that_appears_is_not_followed() {
        let mut rig = Rig::per_directory(|root| write(root, ".gitignore", "build/\n"));
        rig.forget_registered();
        write(rig.root(), "build/out.js", "x");
        rig.event(EventKind::Create(CreateKind::Folder), "build");
        assert!(rig.watched().is_empty());
        rig.heard();
        assert!(rig.rescans.is_empty());
    }

    /// Past the OS's limit nothing is lost: what couldn't be watched is listed,
    /// for a snapshot to check on the disk.
    #[test]
    fn past_the_oss_limit_what_is_left_is_listed_as_uncovered() {
        let rig = Rig::build(
            |root| {
                write(root, "a/f", "");
                write(root, "b/f", "");
            },
            true,
            Some(2),
        );
        assert_eq!(rig.watched().len(), 2);
        let uncovered = rig.follower.uncovered.list();
        assert_eq!(uncovered.len(), 1, "{uncovered:?}");
        assert!(uncovered[0].starts_with(rig.root()));
    }

    /// With one watch on the root, a new folder needs no follower: the OS
    /// reports what's in it.
    #[test]
    fn a_root_watch_hears_a_new_folders_files_with_no_follower() {
        let mut rig = Rig::root_watch(|_| {});
        rig.event(EventKind::Create(CreateKind::Folder), "new");
        rig.modified("new/deep/f.txt");
        assert_eq!(rig.heard(), ["new", "new/deep/f.txt"]);
        assert!(rig.rescans.is_empty());
        assert!(rig.watched().is_empty());
    }

    /// A build writing thousands of files under an ignored folder inside a
    /// kept directory (`.claude/worktrees/x/target`) never reaches a
    /// subscriber. What isn't ignored gets through, after the burst.
    #[test]
    fn a_burst_under_an_ignored_folder_never_reaches_a_subscriber() {
        let mut rig = Rig::root_watch(|root| {
            write(root, ".git/info/exclude", "target/\n");
            std::fs::create_dir_all(root.join(".claude/worktrees/x/target")).unwrap();
        });
        for i in 0..4000 {
            rig.modified(&format!(".claude/worktrees/x/target/o{i}.o"));
        }
        rig.modified(".claude/worktrees/x/lib.rs");
        assert_eq!(rig.heard(), [".claude/worktrees/x/lib.rs"]);
    }

    /// A build tool's `.gitignore` under an ignored folder (`target/.gitignore`)
    /// rules nothing: no rescan, no rules forgotten.
    #[test]
    fn an_ignore_file_in_an_ignored_folder_is_no_ones() {
        let mut rig = Rig::root_watch(|root| write(root, ".gitignore", "target/\n"));
        rig.modified("target/debug/.gitignore");
        assert!(rig.heard().is_empty());
        assert!(rig.rescans.is_empty());
    }

    /// A `.gitignore` edit takes effect at once: a file it starts ignoring
    /// goes quiet, one it stops ignoring is heard, and what it stopped
    /// ignoring is looked for on the disk under the directory it rules.
    #[test]
    fn a_gitignore_edit_takes_effect_without_a_restart() {
        let mut rig = Rig::root_watch(|root| {
            std::fs::create_dir(root.join("src")).unwrap();
            write(root, "built/out.js", "x");
            write(root, "src/.gitignore", "a.txt\n");
            write(root, ".gitignore", "built/\n");
        });
        rig.modified("src/a.txt");
        assert!(rig.heard().is_empty(), "ignored");

        write(rig.root(), "src/.gitignore", "");
        rig.modified("src/.gitignore");
        assert!(rig.heard().is_empty(), "the rescan covers the edit itself");
        assert_eq!(rig.rescans, ["src"], "only what it rules");
        rig.modified("src/a.txt");
        assert_eq!(rig.heard(), ["src/a.txt"]);

        write(rig.root(), ".gitignore", "");
        rig.modified(".gitignore");
        rig.heard();
        assert_eq!(rig.rescans, [""], "the root's rules rule the root");
        rig.modified("built/out.js");
        assert_eq!(rig.heard(), ["built/out.js"]);
    }

    /// With one watch per directory, a rule change moves them too.
    #[test]
    fn a_gitignore_edit_moves_per_directory_watches() {
        let mut rig = Rig::per_directory(|root| {
            write(root, "built/deep/out.js", "x");
            write(root, "src/a.rs", "");
            write(root, ".gitignore", "built/\n");
        });
        assert_eq!(rig.watched(), ["", "src"]);
        rig.forget_registered();

        write(rig.root(), ".gitignore", "");
        rig.modified(".gitignore");
        assert_eq!(rig.watched(), ["built", "built/deep"]);
        rig.forget_registered();

        write(rig.root(), ".gitignore", "src/\n");
        rig.modified(".gitignore");
        assert_eq!(
            rig.registrar.lock().unwrap().unwatched,
            [rig.root().join("src")]
        );
        assert!(rig.watched().is_empty());
    }

    /// The OS may report a path under another spelling of the root (macOS
    /// reports `/private/var` for `/var`, and a symlink's target): a subscriber
    /// hears it in the root's own.
    #[test]
    fn a_path_is_heard_in_the_roots_own_spelling() {
        let mut rig = Rig::root_watch(|_| {});
        let root = rig.root().to_path_buf();
        rig.router
            .route
            .as_mut()
            .unwrap()
            .roots
            .push(PathBuf::from("/other/spelling"));
        rig.router
            .handle(Ok(Event::new(EventKind::Create(CreateKind::File))
                .add_path(PathBuf::from("/other/spelling/src/a.rs"))));
        let batch = rig.heard.try_batch().unwrap();
        assert_eq!(batch.paths[0].0, root.join("src/a.rs"));
    }

    /// A read changes nothing: no event, or a consumer that reads in answer to
    /// an event hears its own read, forever.
    #[test]
    fn a_read_raises_no_event() {
        let mut rig = Rig::root_watch(|_| {});
        rig.event(
            EventKind::Access(notify::event::AccessKind::Read),
            "read.txt",
        );
        assert!(rig.heard().is_empty());
    }

    fn dropped_events(rel: Option<&Path>) -> Event {
        let event = Event::new(EventKind::Other).set_flag(Flag::Rescan);
        match rel {
            Some(path) => event.add_path(path.to_path_buf()),
            None => event,
        }
    }

    /// The OS says it dropped events: subscribers check the disk there, and
    /// an ignore file that changed unheard is read again.
    #[test]
    fn dropped_events_are_a_rescan_where_the_os_says() {
        let mut rig = Rig::root_watch(|root| {
            std::fs::create_dir(root.join("src")).unwrap();
            write(root, "src/.gitignore", "old\n");
        });
        rig.modified("src/old");
        assert!(rig.heard().is_empty(), "ignored");
        write(rig.root(), "src/.gitignore", "");

        let src = rig.root().join("src");
        rig.router.handle(Ok(dropped_events(Some(&src))));
        rig.heard();
        assert_eq!(rig.rescans, ["src"]);
        rig.modified("src/old");
        assert_eq!(rig.heard(), ["src/old"], "the lost edit counts");
    }

    /// With no path, or one outside the watch (macOS may name a parent), it is
    /// the whole root; so is a watch error.
    #[test]
    fn dropped_events_elsewhere_or_a_watch_error_rescan_the_root() {
        let mut rig = Rig::root_watch(|_| {});
        for res in [
            Ok(dropped_events(None)),
            Ok(dropped_events(Some(Path::new("/elsewhere")))),
            Err(notify::Error::generic("gone")),
        ] {
            rig.router.handle(res);
            rig.heard();
            assert_eq!(rig.rescans, [""]);
        }
    }

    /// A build overflowing the OS's queue under an ignored folder concerns
    /// no one.
    #[test]
    fn dropped_events_under_an_ignored_folder_are_no_ones() {
        let mut rig = Rig::root_watch(|root| write(root, ".gitignore", "target/\n"));
        let target = rig.root().join("target/deep");
        rig.router.handle(Ok(dropped_events(Some(&target))));
        rig.heard();
        assert!(rig.rescans.is_empty());
    }
}
