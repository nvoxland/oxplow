//! What a subscriber has not read yet: a set, not a queue. A path written a
//! thousand times is one entry, so a slow subscriber costs memory bounded by
//! what changed, and nothing is ever dropped silently: past a cap the paths
//! collapse into a rescan of the directories that hold them. A subscriber
//! answers a rescan by checking the disk under that directory, so whatever a
//! watch misses (an OS that dropped events, a watch that failed) is found.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tokio::sync::Notify;

use crate::WatchEventKind;

/// How many paths a subscriber holds before they collapse into rescans.
const MAX_PENDING_PATHS: usize = 20_000;

/// What changed since a subscriber last asked.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Batch {
    /// Each changed path, once, with what last happened to it.
    pub paths: Vec<(PathBuf, WatchEventKind)>,
    /// Directories whose changes were not all heard: check the disk under
    /// each. None is inside another.
    pub rescans: Vec<PathBuf>,
}

impl Batch {
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty() && self.rescans.is_empty()
    }
}

#[derive(Default)]
struct State {
    paths: HashMap<PathBuf, WatchEventKind>,
    rescans: BTreeSet<PathBuf>,
    closed: bool,
}

/// The side the watch writes.
#[derive(Default)]
pub(crate) struct Pending {
    state: Mutex<State>,
    wake: Notify,
    /// The directories a path can collapse to: a rescan is of the top-level
    /// directory under one of these.
    roots: Vec<PathBuf>,
}

impl Pending {
    pub(crate) fn new(roots: Vec<PathBuf>) -> Arc<Self> {
        Arc::new(Self {
            roots,
            ..Self::default()
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn change(&self, path: &Path, kind: &WatchEventKind) {
        let mut state = self.lock();
        if state.rescans.iter().any(|dir| path.starts_with(dir)) {
            return;
        }
        match state.paths.get_mut(path) {
            Some(last) => last.clone_from(kind),
            None => {
                state.paths.insert(path.to_path_buf(), kind.clone());
            }
        }
        if state.paths.len() > MAX_PENDING_PATHS {
            self.collapse(&mut state);
        }
        drop(state);
        self.wake.notify_one();
    }

    pub(crate) fn rescan(&self, dir: &Path) {
        let mut state = self.lock();
        insert_rescan(&mut state, dir.to_path_buf());
        drop(state);
        self.wake.notify_one();
    }

    pub(crate) fn close(&self) {
        self.lock().closed = true;
        self.wake.notify_one();
    }

    fn collapse(&self, state: &mut State) {
        let paths: Vec<PathBuf> = state.paths.drain().map(|(path, _)| path).collect();
        for path in paths {
            insert_rescan(state, self.top_of(path));
        }
    }

    /// The top-level directory holding `path`: the root itself for a file
    /// directly in it, and `path`'s parent if it is under no root.
    fn top_of(&self, path: PathBuf) -> PathBuf {
        for root in &self.roots {
            if let Ok(rel) = path.strip_prefix(root) {
                let mut parts = rel.components();
                let first = parts.next();
                return match (first, parts.next()) {
                    (Some(first), Some(_)) => root.join(first),
                    _ => root.clone(),
                };
            }
        }
        path.parent().map(Path::to_path_buf).unwrap_or(path)
    }
}

/// Everyone listening to one watch.
#[derive(Clone, Default)]
pub(crate) struct Subscribers {
    /// What a path can collapse to (see [`Pending::roots`]).
    roots: Vec<PathBuf>,
    /// What a rescan of everything checks: each directory once, in one
    /// spelling.
    whole: Vec<PathBuf>,
    list: Arc<Mutex<Vec<Weak<Pending>>>>,
}

impl Subscribers {
    pub(crate) fn new(roots: Vec<PathBuf>, whole: Vec<PathBuf>) -> Self {
        Self {
            roots,
            whole,
            list: Arc::default(),
        }
    }

    pub(crate) fn subscribe(&self) -> Subscription {
        let pending = Pending::new(self.roots.clone());
        let mut list = self.list.lock().unwrap_or_else(|e| e.into_inner());
        list.retain(|p| p.strong_count() > 0);
        list.push(Arc::downgrade(&pending));
        Subscription::new(pending)
    }

    fn each(&self, f: impl Fn(&Pending)) {
        let list = self.list.lock().unwrap_or_else(|e| e.into_inner());
        for pending in list.iter().filter_map(Weak::upgrade) {
            f(&pending);
        }
    }

    pub(crate) fn change(&self, path: &Path, kind: &WatchEventKind) {
        self.each(|p| p.change(path, kind));
    }

    pub(crate) fn rescan(&self, dir: &Path) {
        self.each(|p| p.rescan(dir));
    }

    pub(crate) fn rescan_all(&self) {
        for dir in &self.whole {
            self.rescan(dir);
        }
    }

    pub(crate) fn close(&self) {
        self.each(Pending::close);
    }
}

/// Keeps `rescans` free of a directory inside another.
fn insert_rescan(state: &mut State, dir: PathBuf) {
    if state.rescans.iter().any(|have| dir.starts_with(have)) {
        return;
    }
    state.rescans.retain(|have| !have.starts_with(&dir));
    state.paths.retain(|path, _| !path.starts_with(&dir));
    state.rescans.insert(dir);
}

/// A subscriber's view of a watch. Its methods are safe to cancel (in a
/// `select!`): what they have not returned stays pending.
pub struct Subscription {
    pending: Arc<Pending>,
}

impl Subscription {
    pub(crate) fn new(pending: Arc<Pending>) -> Self {
        Self { pending }
    }

    /// A subscription nothing writes to and that never ends: for a consumer
    /// whose watch could not start.
    pub fn idle() -> Self {
        Self {
            pending: Arc::default(),
        }
    }

    fn take(&self) -> Result<Batch, Closed> {
        let mut state = self.pending.lock();
        let batch = Batch {
            paths: state.paths.drain().collect(),
            rescans: std::mem::take(&mut state.rescans).into_iter().collect(),
        };
        if batch.is_empty() && state.closed {
            return Err(Closed);
        }
        Ok(batch)
    }

    /// What changed, as soon as anything has; `None` once the watch is gone
    /// and everything has been read.
    pub async fn next_batch(&mut self) -> Option<Batch> {
        self.next_batch_after(Duration::ZERO).await
    }

    /// What changed, `window` after the first change of a burst: a `git
    /// checkout` or a save-storm is one batch, not one per touched file. This
    /// is what most of the system should use.
    pub async fn next_batch_after(&mut self, window: Duration) -> Option<Batch> {
        loop {
            let (has, closed) = {
                let state = self.pending.lock();
                let has = !state.paths.is_empty() || !state.rescans.is_empty();
                (has, state.closed)
            };
            if !has {
                if closed {
                    return None;
                }
                self.pending.wake.notified().await;
                continue;
            }
            if !window.is_zero() {
                tokio::time::sleep(window).await;
            }
            let batch = self.take().ok()?;
            if !batch.is_empty() {
                return Some(batch);
            }
        }
    }

    /// What changed so far, without waiting.
    pub fn try_batch(&mut self) -> Option<Batch> {
        self.take().ok().filter(|batch| !batch.is_empty())
    }
}

struct Closed;

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::timeout;

    const MODIFIED: WatchEventKind = WatchEventKind::Modified;

    fn sub(roots: &[&str]) -> (Arc<Pending>, Subscription) {
        let pending = Pending::new(roots.iter().map(PathBuf::from).collect());
        (pending.clone(), Subscription::new(pending))
    }

    fn sorted(batch: Batch) -> Vec<PathBuf> {
        let mut paths: Vec<_> = batch.paths.into_iter().map(|(path, _)| path).collect();
        paths.sort();
        paths
    }

    /// A path written many times is one entry, with what happened last.
    #[test]
    fn a_path_written_many_times_is_one_entry() {
        let (pending, mut sub) = sub(&["/r"]);
        pending.change(Path::new("/r/a"), &WatchEventKind::Created);
        for _ in 0..1000 {
            pending.change(Path::new("/r/a"), &MODIFIED);
        }
        pending.change(Path::new("/r/b"), &MODIFIED);
        let batch = sub.try_batch().unwrap();
        assert_eq!(batch.paths.len(), 2);
        assert!(batch.paths.contains(&(PathBuf::from("/r/a"), MODIFIED)));
        assert!(sub.try_batch().is_none(), "read once");
    }

    /// Past the cap nothing is lost and memory stays bounded: the paths become
    /// rescans of the top-level directories that held them.
    #[test]
    fn past_the_cap_paths_collapse_into_rescans_of_their_top_directories() {
        let (pending, mut sub) = sub(&["/r"]);
        pending.change(Path::new("/r/at-root.txt"), &MODIFIED);
        for i in 0..=MAX_PENDING_PATHS {
            let top = if i % 2 == 0 { "a" } else { "b" };
            pending.change(&PathBuf::from(format!("/r/{top}/deep/{i}")), &MODIFIED);
        }
        let batch = sub.try_batch().unwrap();
        assert!(batch.paths.is_empty());
        assert_eq!(batch.rescans, [PathBuf::from("/r")]);
    }

    #[test]
    fn past_the_cap_with_no_file_at_the_root_only_the_top_directories_rescan() {
        let (pending, mut sub) = sub(&["/r"]);
        for i in 0..=MAX_PENDING_PATHS {
            let top = if i % 2 == 0 { "a" } else { "b" };
            pending.change(&PathBuf::from(format!("/r/{top}/deep/{i}")), &MODIFIED);
        }
        let batch = sub.try_batch().unwrap();
        assert_eq!(
            batch.rescans,
            [PathBuf::from("/r/a"), PathBuf::from("/r/b")]
        );
    }

    /// A rescan covers what is under it, so a path inside one needs no entry
    /// of its own, and no rescan sits inside another.
    #[test]
    fn a_rescan_covers_what_is_under_it() {
        let (pending, mut sub) = sub(&["/r"]);
        pending.change(Path::new("/r/a/one"), &MODIFIED);
        pending.change(Path::new("/r/keep"), &MODIFIED);
        pending.rescan(Path::new("/r/a/deep"));
        pending.rescan(Path::new("/r/a"));
        pending.rescan(Path::new("/r/a/deep/er"));
        pending.change(Path::new("/r/a/two"), &MODIFIED);
        let batch = sub.try_batch().unwrap();
        assert_eq!(batch.rescans, [PathBuf::from("/r/a")]);
        assert_eq!(sorted(batch), [PathBuf::from("/r/keep")]);
    }

    /// A burst inside the window is one batch. Time is paused, so the window
    /// is exact.
    #[tokio::test(start_paused = true)]
    async fn a_burst_inside_the_window_is_one_batch() {
        let (pending, mut sub) = sub(&["/r"]);
        pending.change(Path::new("/r/a"), &MODIFIED);
        let writer = {
            let pending = pending.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(100)).await;
                pending.change(Path::new("/r/b"), &MODIFIED);
            })
        };
        let batch = sub
            .next_batch_after(Duration::from_millis(150))
            .await
            .unwrap();
        writer.await.unwrap();
        assert_eq!(
            sorted(batch),
            [PathBuf::from("/r/a"), PathBuf::from("/r/b")]
        );
        assert!(
            timeout(Duration::from_secs(1), sub.next_batch())
                .await
                .is_err(),
            "nothing more"
        );
    }

    /// A waiter is woken, and giving up on the wait loses nothing.
    #[tokio::test(start_paused = true)]
    async fn a_cancelled_wait_loses_nothing() {
        let (pending, mut sub) = sub(&["/r"]);
        pending.change(Path::new("/r/a"), &MODIFIED);
        assert!(timeout(
            Duration::from_millis(10),
            sub.next_batch_after(Duration::from_millis(100))
        )
        .await
        .is_err());
        assert_eq!(
            sorted(sub.next_batch().await.unwrap()),
            [PathBuf::from("/r/a")]
        );
    }

    /// The watch going away ends the subscription, after what it had.
    #[tokio::test]
    async fn closing_ends_it_after_what_is_pending() {
        let (pending, mut sub) = sub(&["/r"]);
        pending.change(Path::new("/r/a"), &MODIFIED);
        pending.close();
        assert!(sub.next_batch().await.is_some());
        assert!(sub.next_batch().await.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn an_idle_subscription_waits_forever() {
        let mut sub = Subscription::idle();
        assert!(timeout(Duration::from_secs(3600), sub.next_batch())
            .await
            .is_err());
    }
}
