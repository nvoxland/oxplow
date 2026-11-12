//! The tree a capability implementation's script reads with `tree.read`
//! (`.context/extensions-commands.md` "Implementations"): the stream's
//! worktree **as it is on disk now**, not a snapshot of it. A snapshots
//! implementation marks the worktree to make a snapshot, so the snapshot
//! tree (what a collector reads, [`crate::services::TreeSource`] of
//! `MetricsService`) would be the previous mark's.
//!
//! It follows the stream's capture filter as a tree is judged
//! (`WorkspaceFilter::ignore_in_trees`: `.git`, `.oxplow` but for what git
//! would see, `generated:`, `.gitignore`) so a script sees the files oxplow tracks, and keeps to text:
//! a file over the size cap, with a NUL byte or not valid UTF-8 is left out
//! (a script holds text, and a hash it computes of a file must be of its
//! exact bytes). A path the run's `tree.read:<path>` bounds is filtered by
//! `Wanted::build`, as for every tree.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxplow_domain::events::Anchors;
use oxplow_fs_watch::WorkspaceFilter;

use crate::services::{Tree, TreeSource};
use crate::snapshot_capture_registry::SnapshotCaptureRegistry;

/// A file larger than this is not in the tree when no capture service
/// states its own cap.
const DEFAULT_MAX_BYTES: u64 = 1_048_576;

pub struct WorktreeTree {
    captures: SnapshotCaptureRegistry,
    /// The primary worktree: where a stream with no capture service reads.
    project_dir: PathBuf,
}

impl WorktreeTree {
    pub fn new(captures: SnapshotCaptureRegistry, project_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            captures,
            project_dir,
        })
    }
}

#[async_trait::async_trait]
impl TreeSource for WorktreeTree {
    /// The worktree of the run's stream, else the primary stream's, else the
    /// project's.
    async fn tree(&self, anchors: &Anchors) -> Result<Tree, String> {
        let capture = anchors
            .stream_id
            .and_then(|stream| self.captures.get(&stream))
            .or_else(|| self.captures.primary());
        let (root, filter, max) = match capture {
            Some(capture) => (
                capture.project_dir().to_path_buf(),
                capture.workspace_filter(),
                self.captures.max_file_bytes(),
            ),
            None => (
                self.project_dir.clone(),
                WorkspaceFilter::for_project(
                    &self.project_dir,
                    Vec::<String>::new(),
                    Vec::<String>::new(),
                ),
                DEFAULT_MAX_BYTES,
            ),
        };
        tokio::task::spawn_blocking(move || {
            let mut files = std::collections::HashMap::new();
            read_dir(&root, &root, &filter, max, &mut files);
            Arc::new(files)
        })
        .await
        .map_err(|e| format!("`tree.read`: the worktree could not be read: {e}"))
    }
}

/// Every kept text file under `dir`, by its path from `root`.
fn read_dir(
    root: &Path,
    dir: &Path,
    filter: &WorkspaceFilter,
    max: u64,
    out: &mut std::collections::HashMap<String, String>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        // The filter judges a path from the worktree's root.
        let Ok(from_root) = path.strip_prefix(root) else {
            continue;
        };
        if filter.ignore_in_trees(from_root, kind.is_dir()) {
            continue;
        }
        if kind.is_dir() {
            read_dir(root, &path, filter, max, out);
        } else if kind.is_file() {
            let Some(rel) = from_root.to_str() else {
                continue;
            };
            if std::fs::metadata(&path).map_or(true, |m| m.len() > max) {
                continue;
            }
            if let Ok(text) = std::fs::read(&path)
                .map_err(drop)
                .and_then(|bytes| String::from_utf8(bytes).map_err(drop))
            {
                if !text.contains('\0') {
                    out.insert(rel.replace('\\', "/"), text);
                }
            }
        }
    }
}
