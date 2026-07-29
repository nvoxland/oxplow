//! Crash-safe writes for the small global JSON documents
//! (`session.json`, `recent-projects.json`).
//!
//! Both used to truncate the target and write into it. A process dying
//! in that window — a crash, a `kill`, the machine losing power — left
//! an empty or half-written file, and both stores treat a parse failure
//! as "empty document". So the failure mode was silent data loss: the
//! restore set or the whole recents list simply gone (tsk253).
//!
//! Writing to a sibling temp file and renaming over the target fixes
//! that: `rename` is atomic on POSIX, so a reader sees the old document
//! or the new one and never a partial one.
//!
//! **Lock a different file than the one you replace.** `rename` swaps
//! the inode, so an advisory lock held on the target is a lock on an
//! inode that is no longer the file — two processes could each "hold
//! the lock" on different inodes. The stores lock a `<name>.lock`
//! sidecar, which is never replaced.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Distinguishes temp files from concurrent writers in one process; the
/// pid distinguishes them across processes.
static SEQ: AtomicU64 = AtomicU64::new(0);

/// Replace `path`'s contents with `bytes`, atomically.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;

    // The temp file must live in the target's directory: `rename` is
    // only atomic within a filesystem, and the system temp dir is
    // routinely a different one.
    let tmp = temp_sibling(path, parent);
    let write = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        // Rename ordering says nothing about the *contents* reaching
        // disk; without this a crash right after can leave the new name
        // pointing at an empty file.
        f.sync_all()
    })();
    if let Err(e) = write {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

fn temp_sibling(path: &Path, parent: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "doc".to_string());
    parent.join(format!(
        ".{name}.tmp-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Path of the advisory-lock sidecar guarding `path`'s read-modify-write.
pub(crate) fn lock_path(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_os_string();
    p.push(".lock");
    PathBuf::from(p)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    /// The point of the exercise: the target is *replaced*, never opened
    /// and truncated, so there is no window in which a reader (or a
    /// crash) can see a partial document.
    #[cfg(unix)]
    #[test]
    fn writing_replaces_the_file_rather_than_mutating_it() {
        use std::os::unix::fs::MetadataExt;
        let dir = tempdir().unwrap();
        let path = dir.path().join("doc.json");

        write_atomic(&path, b"first").unwrap();
        let first_inode = std::fs::metadata(&path).unwrap().ino();

        write_atomic(&path, b"second").unwrap();
        let second_inode = std::fs::metadata(&path).unwrap().ino();

        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        assert_ne!(
            first_inode, second_inode,
            "an in-place rewrite would keep the inode — and with it the \
             truncated-file window this exists to close"
        );
    }

    #[test]
    fn no_temp_files_are_left_behind() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("doc.json");
        for i in 0..5 {
            write_atomic(&path, format!("{i}").as_bytes()).unwrap();
        }
        let strays: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
            .filter(|n| n.contains(".tmp-"))
            .collect();
        assert!(strays.is_empty(), "left temp files behind: {strays:?}");
    }

    /// Concurrent writers must not collide on one temp name.
    #[test]
    fn parallel_writes_all_land() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("doc.json");
        std::thread::scope(|s| {
            for i in 0..8 {
                let p = path.clone();
                s.spawn(move || write_atomic(&p, format!("writer {i}").as_bytes()).unwrap());
            }
        });
        // Whoever won, the file is one writer's complete output.
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.starts_with("writer "), "partial content: {body:?}");
    }

    #[test]
    fn lock_path_is_a_sidecar_not_the_target() {
        let p = Path::new("/tmp/state/session.json");
        assert_eq!(lock_path(p), PathBuf::from("/tmp/state/session.json.lock"));
    }
}
