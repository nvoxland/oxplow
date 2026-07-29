//! Global session store: the set of project directories that
//! currently have an open window.
//!
//! The shell owns every window, so it simply knows the set and writes
//! it here (`replace`) whenever a project window opens or closes. On a
//! bare launch the startup path reads `list()` and reopens whatever is
//! still a project on disk — i.e. the windows that were open at last
//! exit. Closing the last window, or quitting, deliberately leaves the
//! set alone so it is what comes back.
//!
//! A dev build and an installed build can share one config dir, so
//! every read-modify-write is wrapped in a cross-process `fs2`
//! exclusive lock — taken on a `.lock` sidecar, since the document
//! itself is replaced by `rename` (see `atomic.rs`).

use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
struct SessionDoc {
    /// Canonical paths of project dirs with a live window.
    #[serde(default)]
    open: Vec<String>,
}

/// Handle to the global `session.json` open-window set.
#[derive(Debug, Clone)]
pub struct SessionProjects {
    json_path: PathBuf,
}

impl SessionProjects {
    pub fn new(json_path: impl Into<PathBuf>) -> Self {
        Self {
            json_path: json_path.into(),
        }
    }

    /// Project dirs currently recorded as open (canonical paths).
    pub fn list(&self) -> Vec<String> {
        self.with_locked(|doc| doc.open.clone()).unwrap_or_default()
    }

    /// Record `dir` as having an open window (dedup by canonical path).
    pub fn add(&self, dir: impl AsRef<Path>) {
        let canonical = canonicalize(dir.as_ref());
        let _ = self.with_locked(|doc| {
            if !doc.open.contains(&canonical) {
                doc.open.push(canonical.clone());
            }
        });
    }

    /// Drop `dir` from the open set (its window was closed).
    pub fn remove(&self, dir: impl AsRef<Path>) {
        let canonical = canonicalize(dir.as_ref());
        let _ = self.with_locked(|doc| {
            doc.open.retain(|p| p != &canonical);
        });
    }

    /// Replace the entire open set with `dirs` (canonicalized). Used to
    /// re-snapshot the live window set on a fresh (non-restore) boot, so
    /// stale entries from previous sessions don't accumulate.
    pub fn replace(&self, dirs: &[std::path::PathBuf]) {
        let canon: Vec<String> = dirs.iter().map(|d| canonicalize(d.as_path())).collect();
        let _ = self.with_locked(move |doc| {
            doc.open = canon;
        });
    }

    /// Take the exclusive cross-process lock, read+parse the doc, run
    /// `f` (which may mutate it), and — if mutated — write it back
    /// atomically. Returns `f`'s result. Any IO/parse failure degrades
    /// to a default (empty) doc so a corrupt file never wedges window
    /// tracking.
    ///
    /// The lock is taken on a `.lock` sidecar rather than on the
    /// document: the write replaces the document by `rename`, and a lock
    /// held on a replaced inode no longer excludes anyone (see
    /// `atomic.rs`).
    fn with_locked<R>(&self, f: impl FnOnce(&mut SessionDoc) -> R) -> std::io::Result<R> {
        if let Some(parent) = self.json_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(crate::atomic::lock_path(&self.json_path))?;
        lock.lock_exclusive()?;

        let raw = std::fs::read_to_string(&self.json_path).unwrap_or_default();
        let before = raw.clone();
        let mut doc: SessionDoc = serde_json::from_str(&raw).unwrap_or_default();

        let result = f(&mut doc);

        // Only rewrite when the serialized form actually changed.
        if let Ok(after) = serde_json::to_string_pretty(&doc) {
            if after != before {
                if let Err(e) = crate::atomic::write_atomic(&self.json_path, after.as_bytes()) {
                    tracing::warn!(error = %e, path = %self.json_path.display(), "failed to write session");
                }
            }
        }
        let _ = FileExt::unlock(&lock);
        Ok(result)
    }
}

/// Canonicalize for stable dedup across symlinks; fall back to the
/// lexical path string if it can't be resolved.
fn canonicalize(path: &Path) -> String {
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn store(dir: &tempfile::TempDir) -> SessionProjects {
        SessionProjects::new(dir.path().join("state").join("session.json"))
    }

    #[test]
    fn missing_file_lists_empty() {
        let dir = tempdir().unwrap();
        assert!(store(&dir).list().is_empty());
    }

    #[test]
    fn add_then_list_then_remove() {
        let dir = tempdir().unwrap();
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        let s = store(&dir);

        s.add(a.path());
        s.add(b.path());
        s.add(a.path()); // dedup
        assert_eq!(s.list().len(), 2);

        let canon_a = std::fs::canonicalize(a.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        s.remove(a.path());
        let remaining = s.list();
        assert_eq!(remaining.len(), 1);
        assert!(!remaining.contains(&canon_a));
    }

    #[test]
    fn replace_overwrites_whole_set() {
        let dir = tempdir().unwrap();
        let a = tempdir().unwrap();
        let b = tempdir().unwrap();
        let c = tempdir().unwrap();
        let s = store(&dir);
        s.add(a.path());
        s.add(b.path());
        assert_eq!(s.list().len(), 2);

        // Re-snapshot to a different live set: A and B drop, C remains.
        s.replace(&[c.path().to_path_buf()]);
        let list = s.list();
        assert_eq!(list.len(), 1);
        let canon_c = std::fs::canonicalize(c.path())
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(list[0], canon_c);
    }

    #[test]
    fn corrupt_file_degrades_to_empty() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("session.json");
        std::fs::write(&path, b"{ not json").unwrap();
        let s = SessionProjects::new(path);
        assert!(s.list().is_empty());
        // Still writable after a corrupt read.
        let proj = tempdir().unwrap();
        s.add(proj.path());
        assert_eq!(s.list().len(), 1);
    }
}
