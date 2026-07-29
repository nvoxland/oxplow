//! Global recent-projects store.
//!
//! Unlike [`crate::load_project_config`] (per-project `.oxplow/project.yaml`),
//! this is **global** app state: the list of project directories the
//! launcher offers to reopen. It lives outside any project's
//! `.oxplow/` dir — the IPC layer resolves the file path via Tauri's
//! path resolver (e.g. `~/Library/Application Support/net.voxland.oxplow/
//! recent-projects.json` on macOS) and hands it to [`RecentProjects::new`].
//!
//! The on-disk shape and read/write discipline mirror the LSP installer
//! manifest in `oxplow-app`: serde_json, create-parent-on-write, and a
//! graceful empty default when the file is missing or corrupt.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use fs2::FileExt;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use specta::Type;

/// Most-recent projects kept; older entries fall off the end.
const MAX_RECENT: usize = 20;

/// One entry in the launcher's recent-projects list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct RecentProject {
    /// Canonicalized absolute path to the project directory.
    pub path: String,
    /// Display name — the directory basename.
    pub title: String,
    /// Unix seconds of the most recent open.
    pub last_opened_at: i64,
}

/// On-disk document. Wrapped in a struct (rather than a bare `Vec`)
/// so future fields can be added without breaking the format.
#[derive(Debug, Default, Serialize, Deserialize)]
struct RecentDoc {
    #[serde(default)]
    projects: Vec<RecentProject>,
}

/// Handle to the global recent-projects JSON file. Cheap to clone via
/// `Arc` at the call site.
///
/// Two locks, for two different races. The in-process mutex stops
/// concurrent IPC calls interleaving a read-modify-write. The `.lock`
/// sidecar (advisory, cross-process) stops a second oxplow sharing the
/// config dir — a dev build alongside an installed one — from doing the
/// same; without it, two `record` calls could each read the old list and
/// the later write would drop the other's entry (tsk253).
#[derive(Debug)]
pub struct RecentProjects {
    json_path: PathBuf,
    lock: Mutex<()>,
}

impl RecentProjects {
    pub fn new(json_path: impl Into<PathBuf>) -> Self {
        Self {
            json_path: json_path.into(),
            lock: Mutex::new(()),
        }
    }

    /// Recent projects, most-recently-opened first.
    pub fn list(&self) -> Vec<RecentProject> {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        self.read().projects
    }

    /// Record an open of `path`: move it to the front, bump its
    /// `last_opened_at`, dedup by canonical path, and cap the list.
    pub fn record(&self, path: impl AsRef<Path>) {
        let canonical = canonicalize(path.as_ref());
        let title = basename(&canonical);
        let now = unix_now();

        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let _x = self.lock_file();
        let mut doc = self.read();
        doc.projects.retain(|p| p.path != canonical);
        doc.projects.insert(
            0,
            RecentProject {
                path: canonical,
                title,
                last_opened_at: now,
            },
        );
        doc.projects.truncate(MAX_RECENT);
        self.write(&doc);
    }

    /// Drop `path` from the list (exact string match on the stored
    /// canonical path).
    pub fn remove(&self, path: &str) {
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let _x = self.lock_file();
        let mut doc = self.read();
        doc.projects.retain(|p| p.path != path);
        self.write(&doc);
    }

    /// Take the cross-process advisory lock for the whole
    /// read-modify-write. Best-effort: if the sidecar can't be opened we
    /// proceed unlocked rather than lose the user's recents entirely.
    fn lock_file(&self) -> Option<std::fs::File> {
        if let Some(parent) = self.json_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(crate::atomic::lock_path(&self.json_path))
            .ok()?;
        file.lock_exclusive().ok()?;
        Some(file)
    }

    fn read(&self) -> RecentDoc {
        match std::fs::read(&self.json_path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => RecentDoc::default(),
        }
    }

    fn write(&self, doc: &RecentDoc) {
        if let Some(parent) = self.json_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_vec_pretty(doc) {
            Ok(json) => {
                // Atomic: a truncate-then-write that died midway left an
                // unparseable file, and an unparseable file reads as an
                // empty list — the whole recents list, silently gone.
                if let Err(e) = crate::atomic::write_atomic(&self.json_path, &json) {
                    tracing::warn!(error = %e, path = %self.json_path.display(), "failed to write recent-projects");
                }
            }
            Err(e) => tracing::warn!(error = %e, "failed to serialize recent-projects"),
        }
    }
}

/// Canonicalize for stable dedup across symlinks; fall back to a
/// lexical absolute string if the path can't be resolved (e.g. it was
/// since deleted — `remove`/stale entries still need a stable key).
fn canonicalize(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

fn basename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".to_string())
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every recorded project must survive concurrent writers.
    ///
    /// Each `record` is a read-modify-write of the whole document, so
    /// two of them racing can each read the old list and the later write
    /// drops the other's entry. The in-process mutex covers threads;
    /// the `.lock` sidecar covers a second oxplow sharing the config
    /// dir (tsk253).
    #[test]
    fn concurrent_records_do_not_lose_entries() {
        let dir = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(RecentProjects::new(dir.path().join("recent.json")));
        let projects: Vec<_> = (0..8).map(|_| tempfile::tempdir().unwrap()).collect();

        std::thread::scope(|s| {
            for p in &projects {
                let store = store.clone();
                let path = p.path().to_path_buf();
                s.spawn(move || store.record(&path));
            }
        });

        assert_eq!(
            store.list().len(),
            projects.len(),
            "a lost update dropped someone's entry"
        );
    }

    /// A half-written document reads as an empty list, so the write must
    /// never be observable in a partial state.
    #[cfg(unix)]
    #[test]
    fn writes_replace_the_file_rather_than_truncating_it() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("recent.json");
        let store = RecentProjects::new(path.clone());
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();

        store.record(a.path());
        let first = std::fs::metadata(&path).unwrap().ino();
        store.record(b.path());
        let second = std::fs::metadata(&path).unwrap().ino();

        assert_ne!(first, second, "an in-place rewrite keeps the inode");
        assert_eq!(store.list().len(), 2);
    }
    use tempfile::tempdir;

    fn store(dir: &tempfile::TempDir) -> RecentProjects {
        RecentProjects::new(dir.path().join("state").join("recent-projects.json"))
    }

    #[test]
    fn missing_file_lists_empty() {
        let dir = tempdir().unwrap();
        assert!(store(&dir).list().is_empty());
    }

    #[test]
    fn record_adds_and_lists() {
        let dir = tempdir().unwrap();
        let proj = tempdir().unwrap();
        let s = store(&dir);
        s.record(proj.path());
        let list = s.list();
        assert_eq!(list.len(), 1);
        assert_eq!(
            list[0].title,
            proj.path().file_name().unwrap().to_string_lossy()
        );
        assert!(list[0].last_opened_at > 0);
    }

    #[test]
    fn record_dedups_and_moves_to_front() {
        let dir = tempdir().unwrap();
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        let s = store(&dir);
        s.record(a.path());
        s.record(b.path());
        s.record(a.path()); // re-open A

        let list = s.list();
        assert_eq!(list.len(), 2, "A must not be duplicated");
        let canon_a = std::fs::canonicalize(a.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(list[0].path, canon_a, "most-recent open is first");
    }

    #[test]
    fn caps_at_max_recent() {
        let dir = tempdir().unwrap();
        let s = store(&dir);
        let projects: Vec<_> = (0..MAX_RECENT + 5).map(|_| tempdir().unwrap()).collect();
        for p in &projects {
            s.record(p.path());
        }
        assert_eq!(s.list().len(), MAX_RECENT);
    }

    #[test]
    fn remove_drops_entry() {
        let dir = tempdir().unwrap();
        let proj = tempdir().unwrap();
        let s = store(&dir);
        s.record(proj.path());
        let stored = s.list()[0].path.clone();
        s.remove(&stored);
        assert!(s.list().is_empty());
    }

    #[test]
    fn corrupt_file_lists_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("recent-projects.json");
        std::fs::write(&path, b"{ not valid json").unwrap();
        let s = RecentProjects::new(path);
        assert!(s.list().is_empty());
    }
}
