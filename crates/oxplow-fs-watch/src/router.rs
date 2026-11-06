//! What happens to an OS event: [`Router::handle`] runs on the OS callback
//! thread and decides what reaches the stream, and [`Follower`] moves the
//! watches of a workspace watch. Both are plain code over plain values, so
//! they are tested by handing them made-up events (`tests` below), with no OS.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, RwLock, Weak};

use notify::RecursiveMode;
use tokio::sync::broadcast;

use crate::filter::{is_ignore_file, WorkspaceFilter};
use crate::registrar::Registrar;
use crate::{WatchEvent, WatchEventKind};

/// What the watch callback tells the follower.
pub(crate) enum FollowMsg {
    /// A path directly under the root changed.
    Top(PathBuf),
    /// An ignore file changed: read git's rules again.
    Refilter,
}

/// A workspace watch's half of the router.
pub(crate) struct Route {
    /// The root, as given and canonical: the OS may report either spelling
    /// (macOS reports `/private/var` for `/var`).
    pub(crate) roots: Vec<PathBuf>,
    /// Shared with the follower, which swaps it when an ignore file changes.
    pub(crate) filter: Arc<RwLock<WorkspaceFilter>>,
    /// Off the OS callback thread: registering a watch from it can deadlock
    /// a backend.
    pub(crate) follower: mpsc::Sender<FollowMsg>,
}

/// Decides what an OS event means.
pub(crate) struct Router {
    pub(crate) events: broadcast::Sender<WatchEvent>,
    pub(crate) route: Option<Route>,
}

impl Router {
    pub(crate) fn handle(&self, res: notify::Result<notify::Event>) {
        let Ok(event) = res else { return };
        // A read changes nothing. inotify reports one (open, close
        // after reading) for a file the watcher's own consumer is
        // reading, and a consumer that reads on every event —
        // the config reload — would otherwise hear itself: an
        // endless loop that kept a daemon's boot from finishing.
        if matches!(event.kind, notify::EventKind::Access(_)) {
            return;
        }
        let kind = classify(&event);
        for path in &event.paths {
            if let Some(route) = &self.route {
                if is_ignore_file(path) {
                    // Read git's rules again here, so what is
                    // written next is judged by them; the follower
                    // then moves the watches.
                    let fresh = route
                        .filter
                        .read()
                        .unwrap_or_else(|e| e.into_inner())
                        .refreshed();
                    *route.filter.write().unwrap_or_else(|e| e.into_inner()) = fresh;
                    let _ = route.follower.send(FollowMsg::Refilter);
                } else if ignored_by(&route.filter, &route.roots, path) {
                    // A build writing thousands of files under an ignored
                    // folder (inside a kept directory, where the OS watch
                    // still sees it) would overflow the channel and make
                    // every consumer lag and rescan.
                    continue;
                }
            }
            let _ = self.events.send(WatchEvent {
                path: path.clone(),
                kind: kind.clone(),
            });
            if let Some(route) = &self.route {
                if path
                    .parent()
                    .is_some_and(|parent| route.roots.iter().any(|r| r == parent))
                {
                    let _ = route.follower.send(FollowMsg::Top(path.clone()));
                }
            }
        }
    }
}

/// Whether the filter ignores `path`, a file under an ignored directory with
/// it. It is judged as what it is now, since a directory-only pattern
/// (`target/`) matches only a directory: macOS reports a directory itself, and
/// one already gone is judged as a file. A path outside every root is not
/// judged.
fn ignored_by(filter: &RwLock<WorkspaceFilter>, roots: &[PathBuf], path: &Path) -> bool {
    let Some(rel) = roots.iter().find_map(|r| path.strip_prefix(r).ok()) else {
        return false;
    };
    filter
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .ignore(rel, path.is_dir())
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

/// A workspace watch's follower: adds the recursive watch for each kept
/// top-level directory that appears, and reports what's already in it.
pub(crate) struct Follower {
    /// The root, as given and canonical.
    pub(crate) roots: Vec<PathBuf>,
    /// Shared with the router.
    pub(crate) filter: Arc<RwLock<WorkspaceFilter>>,
    /// The top-level directories with a recursive watch, by name.
    pub(crate) followed: HashSet<OsString>,
    pub(crate) events: broadcast::Sender<WatchEvent>,
}

impl Follower {
    /// Until the watcher is dropped: that drops the router, and with it the
    /// only sender of `messages`.
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
            FollowMsg::Top(path) => self.top_level_changed(&path, watcher),
            FollowMsg::Refilter => self.refilter(watcher),
        }
    }

    fn ignores(&self, rel: &Path, is_dir: bool) -> bool {
        self.filter
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .ignore(rel, is_dir)
    }

    fn top_level_changed(&mut self, path: &Path, watcher: &Mutex<impl Registrar + ?Sized>) {
        let Some(name) = path.file_name().map(|n| n.to_os_string()) else {
            return;
        };
        let mut w = watcher.lock().unwrap_or_else(|e| e.into_inner());
        if !path.is_dir() {
            // Gone: a later one of the same name is new again.
            if self.followed.remove(&name) {
                let _ = w.unwatch(path);
            }
            return;
        }
        if self.followed.contains(&name) || self.ignores(Path::new(&name), true) {
            return;
        }
        if let Err(e) = w.watch(path, RecursiveMode::Recursive) {
            tracing::warn!(error = %e, ?path, "could not watch a new top-level directory");
            return;
        }
        drop(w);
        self.followed.insert(name.clone());
        self.report_files(path, Path::new(&name));
    }

    /// An ignore file changed (the router has read the rules again): stop
    /// watching a top-level directory they now ignore, and watch (and
    /// report) one they stopped ignoring.
    fn refilter(&mut self, watcher: &Mutex<impl Registrar + ?Sized>) {
        let Some(root) = self.roots.first().cloned() else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(&root) else {
            return;
        };
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            let name = entry.file_name();
            let ignored = self.ignores(Path::new(&name), true);
            let followed = self.followed.contains(&name);
            let mut w = watcher.lock().unwrap_or_else(|e| e.into_inner());
            if ignored && followed {
                let _ = w.unwatch(&entry.path());
                self.followed.remove(&name);
            } else if !ignored && !followed {
                if w.watch(&entry.path(), RecursiveMode::Recursive).is_err() {
                    continue;
                }
                drop(w);
                self.followed.insert(name.clone());
                self.report_files(&entry.path(), Path::new(&name));
            }
        }
    }

    /// Report every kept file under `dir` (`rel` from the root), as the
    /// walks do.
    fn report_files(&self, dir: &Path, rel: &Path) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let rel = rel.join(entry.file_name());
            if self.ignores(&rel, file_type.is_dir()) {
                continue;
            }
            if file_type.is_dir() {
                self.report_files(&path, &rel);
            } else if file_type.is_file() {
                let _ = self.events.send(WatchEvent {
                    path,
                    kind: WatchEventKind::Other,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, DataChange, ModifyKind};
    use notify::{Event, EventKind};
    use tempfile::{tempdir, TempDir};
    use tokio::sync::broadcast::error::TryRecvError;

    /// Records what the follower registers.
    #[derive(Default)]
    struct FakeRegistrar {
        watched: Vec<PathBuf>,
        unwatched: Vec<PathBuf>,
    }

    impl Registrar for FakeRegistrar {
        fn watch(&mut self, path: &Path, _: RecursiveMode) -> notify::Result<()> {
            self.watched.push(path.to_path_buf());
            Ok(())
        }
        fn unwatch(&mut self, path: &Path) -> notify::Result<()> {
            self.unwatched.push(path.to_path_buf());
            Ok(())
        }
    }

    /// A workspace watch with no OS: an event is handed in, and everything
    /// it sets off has happened when [`Self::event`] returns.
    struct Rig {
        dir: TempDir,
        router: Router,
        follower: Follower,
        messages: mpsc::Receiver<FollowMsg>,
        registrar: Mutex<FakeRegistrar>,
        heard: broadcast::Receiver<WatchEvent>,
    }

    impl Rig {
        /// `prepare` makes the tree the filter is built from, and `followed`
        /// names the top-level directories already watched.
        fn new(prepare: impl FnOnce(&Path), followed: &[&str]) -> Self {
            let dir = tempdir().unwrap();
            let root = dir.path().to_path_buf();
            prepare(&root);
            let filter = Arc::new(RwLock::new(WorkspaceFilter::build(
                &root,
                Vec::<String>::new(),
                Vec::<String>::new(),
                None,
            )));
            // Room for the largest burst here: what these tests lose is
            // what the router dropped, never what the channel did.
            let (events, heard) = broadcast::channel(16_384);
            let (follower_tx, messages) = mpsc::channel();
            Self {
                router: Router {
                    events: events.clone(),
                    route: Some(Route {
                        roots: vec![root.clone()],
                        filter: filter.clone(),
                        follower: follower_tx,
                    }),
                },
                follower: Follower {
                    roots: vec![root],
                    filter,
                    followed: followed.iter().map(OsString::from).collect(),
                    events,
                },
                messages,
                registrar: Mutex::new(FakeRegistrar::default()),
                heard,
                dir,
            }
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

        /// The paths heard since the last call, from the root.
        fn heard(&mut self) -> Vec<String> {
            let mut out = Vec::new();
            loop {
                match self.heard.try_recv() {
                    Ok(event) => out.push(
                        event
                            .path
                            .strip_prefix(self.dir.path())
                            .unwrap()
                            .to_string_lossy()
                            .into_owned(),
                    ),
                    Err(TryRecvError::Empty) => return out,
                    Err(e) => panic!("{e}"),
                }
            }
        }

        fn watched(&self) -> Vec<PathBuf> {
            self.registrar.lock().unwrap().watched.clone()
        }
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// `mkdir d && write d/f` races the new watch, and no watch sees the gap:
    /// a top-level directory that appears gets its watch, and the files already
    /// in it are reported. What is written after is heard like anything else.
    #[test]
    fn a_top_level_dir_created_later_is_followed_and_its_files_reported() {
        let mut rig = Rig::new(|_| {}, &[]);
        write(rig.root(), "oxplow/extensions/mine/extension.yaml", "x");
        rig.event(EventKind::Create(CreateKind::Folder), "oxplow");
        let late = rig.root().join("oxplow");
        assert_eq!(rig.watched(), [late]);
        let heard = rig.heard();
        assert!(
            heard.contains(&"oxplow/extensions/mine/extension.yaml".to_string()),
            "{heard:?}"
        );

        rig.modified("oxplow/extensions/mine/page.yaml");
        assert_eq!(rig.heard(), ["oxplow/extensions/mine/page.yaml"]);
    }

    /// A directory that goes is unwatched, so one of the same name is new.
    #[test]
    fn a_top_level_dir_that_goes_is_new_when_it_comes_back() {
        let mut rig = Rig::new(|_| {}, &["built"]);
        rig.modified("built");
        assert_eq!(
            rig.registrar.lock().unwrap().unwatched,
            [rig.root().join("built")]
        );
        std::fs::create_dir(rig.root().join("built")).unwrap();
        rig.event(EventKind::Create(CreateKind::Folder), "built");
        assert_eq!(rig.watched(), [rig.root().join("built")]);
    }

    /// A top-level directory the filter drops stays unwatched when it appears.
    #[test]
    fn a_filtered_top_level_dir_created_later_is_not_followed() {
        let mut rig = Rig::new(|root| write(root, ".gitignore", "build/\n"), &[]);
        write(rig.root(), "build/out.js", "x");
        rig.event(EventKind::Create(CreateKind::Folder), "build");
        assert!(rig.watched().is_empty());
        assert!(rig.heard().is_empty());
    }

    /// A build writing thousands of files under an ignored folder inside a
    /// kept directory (`.claude/worktrees/x/target`) never reaches the
    /// stream. What isn't ignored gets through, after the burst.
    #[test]
    fn a_burst_under_an_ignored_folder_never_reaches_the_stream() {
        let mut rig = Rig::new(
            |root| {
                write(root, ".git/info/exclude", "target/\n");
                std::fs::create_dir_all(root.join(".claude/worktrees/x/target")).unwrap();
            },
            &[".claude"],
        );
        for i in 0..4000 {
            rig.modified(&format!(".claude/worktrees/x/target/o{i}.o"));
        }
        rig.modified(".claude/worktrees/x/lib.rs");
        assert_eq!(rig.heard(), [".claude/worktrees/x/lib.rs"]);
    }

    /// A `.gitignore` edit takes effect at once: a file it starts ignoring
    /// goes quiet, one it stops ignoring is heard, and a top-level directory
    /// it stops ignoring is watched from then on with its files reported.
    #[test]
    fn a_gitignore_edit_takes_effect_without_a_restart() {
        let mut rig = Rig::new(
            |root| {
                std::fs::create_dir(root.join("src")).unwrap();
                write(root, "built/out.js", "x");
                write(root, ".gitignore", "built/\n");
            },
            &["src"],
        );
        rig.modified("src/a.txt");
        assert_eq!(rig.heard(), ["src/a.txt"]);

        write(rig.root(), ".gitignore", "built/\na.txt\n");
        rig.modified(".gitignore");
        assert_eq!(rig.heard(), [".gitignore"], "the edit itself is heard");
        rig.modified("src/a.txt");
        assert!(rig.heard().is_empty(), "ignored now");
        assert!(rig.watched().is_empty(), "built/ is still ignored");

        write(rig.root(), ".gitignore", "");
        rig.modified(".gitignore");
        let heard = rig.heard();
        assert!(heard.contains(&"built/out.js".to_string()), "{heard:?}");
        assert_eq!(rig.watched(), [rig.root().join("built")]);
        rig.modified("src/a.txt");
        assert_eq!(rig.heard(), ["src/a.txt"], "kept again");

        write(rig.root(), ".gitignore", "built/\n");
        rig.modified(".gitignore");
        assert_eq!(
            rig.registrar.lock().unwrap().unwatched,
            [rig.root().join("built")],
            "ignored again: the watch goes"
        );
    }

    /// A read changes nothing: no event, or a consumer that reads in answer to
    /// an event hears its own read, forever.
    #[test]
    fn a_read_raises_no_event() {
        let mut rig = Rig::new(|_| {}, &[]);
        rig.event(
            EventKind::Access(notify::event::AccessKind::Read),
            "read.txt",
        );
        assert!(rig.heard().is_empty());
    }
}
