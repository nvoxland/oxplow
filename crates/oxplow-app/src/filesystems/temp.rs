//! `oxplow:temp`: scratch files oxplow keeps for a while
//! (`.context/filesystems.md`). A flat folder on disk,
//! `<project>/.oxplow/runtime/temp/<name>`, so a restart keeps a file a
//! window still shows. A name is `<stem>.<ext>` (`[A-Za-z0-9_-]+` then
//! `[A-Za-z0-9]+`): no folders, and no revisions — a file is its latest
//! content.
//!
//! It is not `writable`: `oxplow.file.save` never touches it. oxplow's own
//! code writes with [`TempFs::put`] and [`TempFs::remove`] (reached as
//! `Services.temp`).
//!
//! Old files go: [`TempFs::gc`] drops a file idle longer than the TTL, then
//! the oldest until at most `max_files` files and `max_bytes` bytes stay.
//! "Idle" is the file's modified time. A read refreshes it when it is more
//! than a day old, so a file someone keeps opening stays; a younger one is
//! left alone, so reading does not change `stat`'s `modified_at` (a cache
//! keyed by it) more than once a day. It runs after every put and on a
//! five-minute tick.

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use oxplow_domain::filesystem::{
    ByteRange, FileSystem, FsCx, FsEntry, FsError, FsKind, FsStat, TEMP,
};

use super::local::{read_range_on_disk, stat_on_disk};

/// Where the folder is, under a project.
pub const TEMP_DIR: &str = ".oxplow/runtime/temp";

/// A read refreshes a file's idle clock when it is older than this.
const TOUCH_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

/// How the folder is kept small.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// A file idle longer than this goes.
    pub ttl: Duration,
    pub max_files: usize,
    pub max_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            ttl: Duration::from_secs(7 * 24 * 60 * 60),
            max_files: 200,
            max_bytes: 32 * 1024 * 1024,
        }
    }
}

pub struct TempFs {
    dir: PathBuf,
    limits: Limits,
}

impl TempFs {
    pub fn new(project: &std::path::Path) -> Self {
        Self::with_limits(project, Limits::default())
    }

    pub fn with_limits(project: &std::path::Path, limits: Limits) -> Self {
        Self {
            dir: project.join(TEMP_DIR),
            limits,
        }
    }

    /// Replace the file `name` with `bytes`, then drop what is too old or
    /// too much. A file bigger than the byte cap is refused.
    pub async fn put(&self, name: &str, bytes: &[u8]) -> Result<(), FsError> {
        valid_name(name)?;
        if bytes.len() as u64 > self.limits.max_bytes {
            return Err(FsError::Invalid(format!(
                "`{name}` is {} bytes; {TEMP} holds at most {} bytes of files",
                bytes.len(),
                self.limits.max_bytes
            )));
        }
        let (dir, name, bytes) = (self.dir.clone(), name.to_string(), bytes.to_vec());
        blocking(move || {
            std::fs::create_dir_all(&dir)?;
            // Written beside, then renamed in: a reader never sees half.
            let tmp = dir.join(format!(".{name}.tmp"));
            std::fs::write(&tmp, bytes)?;
            std::fs::rename(&tmp, dir.join(&name))
        })
        .await?
        .map_err(storage)?;
        self.gc_blocking(SystemTime::now()).await
    }

    /// Delete the file `name`; one that isn't there is fine.
    pub async fn remove(&self, name: &str) -> Result<(), FsError> {
        valid_name(name)?;
        let path = self.dir.join(name);
        match blocking(move || std::fs::remove_file(path)).await? {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(storage(e)),
            _ => Ok(()),
        }
    }

    /// Drop what is idle past the TTL, then the oldest files until the
    /// count and size caps hold, as of `now`. Returns how many went.
    pub fn gc(&self, now: SystemTime) -> std::io::Result<usize> {
        let mut files = Vec::new();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(e),
        };
        for entry in entries.flatten() {
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let meta = match entry.metadata() {
                Ok(meta) if meta.is_file() && valid_name(&name).is_ok() => meta,
                _ => continue,
            };
            let modified = meta.modified().unwrap_or(UNIX_EPOCH);
            files.push((modified, name, meta.len()));
        }
        // Oldest first; a tie by name keeps the order stable.
        files.sort();
        let mut removed = 0;
        let mut kept = Vec::new();
        for (modified, name, len) in files {
            let idle = now.duration_since(modified).unwrap_or_default();
            if idle > self.limits.ttl && std::fs::remove_file(self.dir.join(&name)).is_ok() {
                removed += 1;
            } else {
                kept.push((name, len));
            }
        }
        let mut count = kept.len();
        let mut bytes: u64 = kept.iter().map(|(_, len)| len).sum();
        for (name, len) in kept {
            if count <= self.limits.max_files && bytes <= self.limits.max_bytes {
                break;
            }
            if std::fs::remove_file(self.dir.join(&name)).is_ok() {
                removed += 1;
                count -= 1;
                bytes -= len;
            }
        }
        Ok(removed)
    }

    async fn gc_blocking(&self, now: SystemTime) -> Result<(), FsError> {
        let this = Self {
            dir: self.dir.clone(),
            limits: self.limits,
        };
        blocking(move || this.gc(now))
            .await?
            .map(|_| ())
            .map_err(storage)
    }

    /// Run [`Self::gc`] off the async threads: the five-minute tick.
    pub async fn sweep(&self) {
        if let Err(e) = self.gc_blocking(SystemTime::now()).await {
            tracing::warn!(error = %e, "sweeping the temp files failed");
        }
    }
}

fn storage(e: std::io::Error) -> FsError {
    FsError::Storage(e.to_string())
}

async fn blocking<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> Result<R, FsError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| FsError::Storage(format!("temp file work didn't finish: {e}")))
}

/// `<stem>.<ext>`: `[A-Za-z0-9_-]+` then `.` then `[A-Za-z0-9]+`.
fn valid_name(name: &str) -> Result<(), FsError> {
    let ok = name.split_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && stem
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
            && !ext.is_empty()
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
    });
    if ok {
        Ok(())
    } else {
        Err(FsError::Invalid(format!(
            "`{name}` isn't a name on {TEMP}: it is `<name>.<ext>` of letters, digits, `_` and `-`, \
             with no folders"
        )))
    }
}

fn no_revision(rev: Option<&str>) -> Result<(), FsError> {
    match rev {
        None => Ok(()),
        Some(rev) => Err(FsError::Invalid(format!(
            "{TEMP} has no revisions; `{rev}` isn't one"
        ))),
    }
}

#[async_trait]
impl FileSystem for TempFs {
    fn id(&self) -> &str {
        TEMP
    }

    fn features(&self) -> &[&'static str] {
        &[]
    }

    async fn stat(
        &self,
        _cx: &FsCx,
        rev: Option<&str>,
        path: &str,
    ) -> Result<Option<FsStat>, FsError> {
        no_revision(rev)?;
        if path.is_empty() {
            return Ok(Some(FsStat {
                kind: FsKind::Dir,
                size: None,
                modified_at: None,
                is_text: None,
                language: None,
            }));
        }
        valid_name(path)?;
        let (dir, path) = (self.dir.clone(), path.to_string());
        blocking(move || stat_on_disk(&dir, &path))
            .await?
            .map_err(storage)
    }

    async fn read(
        &self,
        _cx: &FsCx,
        rev: Option<&str>,
        path: &str,
        range: Option<ByteRange>,
    ) -> Result<Option<Vec<u8>>, FsError> {
        no_revision(rev)?;
        valid_name(path)?;
        let (dir, path) = (self.dir.clone(), path.to_string());
        blocking(move || {
            let file = dir.join(&path);
            let bytes = match range {
                Some(range) => read_range_on_disk(&dir, &path, range)?,
                None if file.is_file() => Some(std::fs::read(&file)?),
                None => None,
            };
            if bytes.is_some() {
                touch_if_old(&file);
            }
            Ok(bytes)
        })
        .await?
        .map_err(storage)
    }

    async fn list(
        &self,
        _cx: &FsCx,
        rev: Option<&str>,
        parent: Option<&str>,
    ) -> Result<Vec<FsEntry>, FsError> {
        no_revision(rev)?;
        match parent {
            None | Some("") => {}
            Some(parent) => {
                let escapes = std::path::Path::new(parent)
                    .components()
                    .any(|c| !matches!(c, std::path::Component::Normal(_)));
                return if escapes {
                    Err(FsError::Invalid(format!(
                        "`{parent}` isn't a folder inside {TEMP}"
                    )))
                } else {
                    // Flat: nothing has a folder to list.
                    Ok(Vec::new())
                };
            }
        }
        let dir = self.dir.clone();
        blocking(move || {
            let mut out = Vec::new();
            let entries = match std::fs::read_dir(&dir) {
                Ok(entries) => entries,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
                Err(e) => return Err(e),
            };
            for entry in entries.flatten() {
                let Ok(name) = entry.file_name().into_string() else {
                    continue;
                };
                if valid_name(&name).is_ok() && entry.metadata().is_ok_and(|m| m.is_file()) {
                    out.push(FsEntry {
                        path: name,
                        kind: FsKind::File,
                        status: None,
                    });
                }
            }
            out.sort_by(|a, b| a.path.cmp(&b.path));
            Ok(out)
        })
        .await?
        .map_err(storage)
    }
}

/// Best effort: a read of a file more than a day old restarts its idle
/// clock.
fn touch_if_old(file: &std::path::Path) {
    let old = std::fs::metadata(file)
        .and_then(|m| m.modified())
        .is_ok_and(|t| t.elapsed().is_ok_and(|age| age > TOUCH_AFTER));
    if old {
        let _ = std::fs::File::options()
            .write(true)
            .open(file)
            .and_then(|f| f.set_modified(SystemTime::now()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem_conformance as conformance;

    fn fs(limits: Limits) -> (tempfile::TempDir, TempFs) {
        let dir = tempfile::tempdir().unwrap();
        let fs = TempFs::with_limits(dir.path(), limits);
        (dir, fs)
    }

    fn cx() -> FsCx {
        FsCx::default()
    }

    fn age(fs: &TempFs, name: &str, by: Duration) {
        let file = std::fs::File::options()
            .write(true)
            .open(fs.dir.join(name))
            .unwrap();
        file.set_modified(SystemTime::now() - by).unwrap();
    }

    async fn names(fs: &TempFs) -> Vec<String> {
        fs.list(&cx(), None, None)
            .await
            .unwrap()
            .into_iter()
            .map(|e| e.path)
            .collect()
    }

    #[tokio::test]
    async fn put_reads_back_and_replaces() {
        let (_d, fs) = fs(Limits::default());
        let cx = cx();
        fs.put("view-1.yaml", b"title: One\n").await.unwrap();
        let read = |range| fs.read(&cx, None, "view-1.yaml", range);
        assert_eq!(read(None).await.unwrap().unwrap(), b"title: One\n");
        assert_eq!(
            read(Some(ByteRange { offset: 7, len: 3 }))
                .await
                .unwrap()
                .unwrap(),
            b"One"
        );
        let stat = fs.stat(&cx, None, "view-1.yaml").await.unwrap().unwrap();
        assert_eq!(
            (stat.kind, stat.size, stat.is_text, stat.language.as_deref()),
            (FsKind::File, Some(11), Some(true), Some("yaml"))
        );
        fs.put("view-1.yaml", b"title: Two\n").await.unwrap();
        assert_eq!(read(None).await.unwrap().unwrap(), b"title: Two\n");
        assert_eq!(names(&fs).await, ["view-1.yaml"], "a put replaces");
        assert_eq!(fs.read(&cx, None, "nope.yaml", None).await.unwrap(), None);
        assert_eq!(fs.stat(&cx, None, "nope.yaml").await.unwrap(), None);
        fs.remove("view-1.yaml").await.unwrap();
        fs.remove("view-1.yaml").await.unwrap();
        assert!(names(&fs).await.is_empty());
    }

    #[tokio::test]
    async fn only_flat_names_are_files() {
        let (_d, fs) = fs(Limits::default());
        for name in [
            "", "x", "x.", ".yaml", "a.b.c", "a/b.txt", "../x.txt", "a b.txt", "a.t-t", "/x.txt",
        ] {
            let invalid = |e: FsError| matches!(e, FsError::Invalid(_));
            if !name.is_empty() {
                assert!(
                    fs.put(name, b"x").await.err().is_some_and(invalid),
                    "put {name:?}"
                );
                assert!(
                    fs.remove(name).await.err().is_some_and(invalid),
                    "remove {name:?}"
                );
                assert!(
                    fs.read(&cx(), None, name, None)
                        .await
                        .err()
                        .is_some_and(invalid),
                    "read {name:?}"
                );
            }
        }
        fs.put("A_b-9.Txt", b"x").await.unwrap();
        assert_eq!(names(&fs).await, ["A_b-9.Txt"]);
    }

    #[tokio::test]
    async fn it_has_no_revisions() {
        let (_d, fs) = fs(Limits::default());
        fs.put("a.txt", b"x").await.unwrap();
        for rev in ["working", "abc", ""] {
            let invalid = |e: FsError| matches!(e, FsError::Invalid(_));
            assert!(fs
                .stat(&cx(), Some(rev), "a.txt")
                .await
                .err()
                .is_some_and(invalid));
            assert!(fs
                .read(&cx(), Some(rev), "a.txt", None)
                .await
                .err()
                .is_some_and(invalid));
            assert!(fs
                .list(&cx(), Some(rev), None)
                .await
                .err()
                .is_some_and(invalid));
        }
    }

    #[tokio::test]
    async fn the_flat_listing_is_the_files() {
        let (_d, fs) = fs(Limits::default());
        assert!(names(&fs).await.is_empty(), "no folder yet");
        fs.put("b.txt", b"b").await.unwrap();
        fs.put("a.txt", b"a").await.unwrap();
        assert_eq!(names(&fs).await, ["a.txt", "b.txt"]);
        assert_eq!(
            fs.list(&cx(), None, Some("")).await.unwrap(),
            fs.list(&cx(), None, None).await.unwrap()
        );
        assert!(fs
            .list(&cx(), None, Some("a.txt"))
            .await
            .unwrap()
            .is_empty());
        let root = fs.stat(&cx(), None, "").await.unwrap().unwrap();
        assert_eq!(root.kind, FsKind::Dir);
    }

    /// The contract's path check applies; the others need folders. A write
    /// is refused as read-only.
    #[tokio::test]
    async fn it_passes_the_conformance_checks_that_fit_a_flat_folder() {
        let (_d, fs) = fs(Limits::default());
        conformance::a_path_never_leaves_the_filesystem(&fs, &cx(), None).await;
        assert!(!fs.has_feature("writable"));
        assert_eq!(
            fs.write(&cx(), "a.txt", b"x").await,
            Err(FsError::ReadOnly(TEMP.to_string()))
        );
        assert!(names(&fs).await.is_empty());
    }

    #[tokio::test]
    async fn a_file_idle_past_the_ttl_goes_on_the_next_put() {
        let (_d, fs) = fs(Limits {
            ttl: Duration::from_secs(100),
            ..Limits::default()
        });
        fs.put("old.txt", b"x").await.unwrap();
        fs.put("fresh.txt", b"x").await.unwrap();
        age(&fs, "old.txt", Duration::from_secs(101));
        age(&fs, "fresh.txt", Duration::from_secs(99));
        fs.put("new.txt", b"x").await.unwrap();
        assert_eq!(names(&fs).await, ["fresh.txt", "new.txt"]);
    }

    #[tokio::test]
    async fn the_count_cap_drops_the_oldest_first() {
        let (_d, fs) = fs(Limits {
            max_files: 3,
            ..Limits::default()
        });
        for (i, name) in ["a.txt", "b.txt", "c.txt"].into_iter().enumerate() {
            fs.put(name, b"x").await.unwrap();
            age(&fs, name, Duration::from_secs(300 - i as u64 * 100));
        }
        fs.put("d.txt", b"x").await.unwrap();
        assert_eq!(
            names(&fs).await,
            ["b.txt", "c.txt", "d.txt"],
            "a.txt was oldest"
        );
        age(&fs, "d.txt", Duration::from_secs(1000));
        fs.put("e.txt", b"x").await.unwrap();
        assert_eq!(
            names(&fs).await,
            ["b.txt", "c.txt", "e.txt"],
            "now d.txt was"
        );
    }

    #[tokio::test]
    async fn the_size_cap_drops_the_oldest_first() {
        let (_d, fs) = fs(Limits {
            max_bytes: 10,
            ..Limits::default()
        });
        fs.put("a.txt", b"1234").await.unwrap();
        age(&fs, "a.txt", Duration::from_secs(200));
        fs.put("b.txt", b"1234").await.unwrap();
        age(&fs, "b.txt", Duration::from_secs(100));
        assert_eq!(names(&fs).await, ["a.txt", "b.txt"]);
        fs.put("c.txt", b"1234").await.unwrap();
        assert_eq!(
            names(&fs).await,
            ["b.txt", "c.txt"],
            "12 bytes > 10: a.txt goes"
        );
        let too_big = fs.put("big.txt", &[b'x'; 11]).await.unwrap_err();
        assert!(matches!(too_big, FsError::Invalid(_)), "{too_big:?}");
        assert_eq!(
            names(&fs).await,
            ["b.txt", "c.txt"],
            "nothing was evicted for it"
        );
    }

    #[tokio::test]
    async fn a_sweep_applies_the_ttl_without_a_put() {
        let (_d, fs) = fs(Limits {
            ttl: Duration::from_secs(100),
            ..Limits::default()
        });
        fs.put("old.txt", b"x").await.unwrap();
        age(&fs, "old.txt", Duration::from_secs(200));
        fs.sweep().await;
        assert!(names(&fs).await.is_empty());
    }

    #[tokio::test]
    async fn a_read_keeps_a_file_someone_opens() {
        let (_d, fs) = fs(Limits {
            ttl: Duration::from_secs(3 * 24 * 60 * 60),
            ..Limits::default()
        });
        fs.put("opened.txt", b"x").await.unwrap();
        fs.put("ignored.txt", b"x").await.unwrap();
        for name in ["opened.txt", "ignored.txt"] {
            age(&fs, name, Duration::from_secs(2 * 24 * 60 * 60));
        }
        fs.read(&cx(), None, "opened.txt", None).await.unwrap();
        // Two days on, only the read one is within the TTL.
        let later = SystemTime::now() + Duration::from_secs(2 * 24 * 60 * 60);
        fs.gc(later).unwrap();
        assert_eq!(names(&fs).await, ["opened.txt"]);
    }

    #[tokio::test]
    async fn a_young_file_is_not_touched_by_a_read() {
        let (_d, fs) = fs(Limits::default());
        fs.put("a.txt", b"x").await.unwrap();
        age(&fs, "a.txt", Duration::from_secs(3600));
        let before = fs
            .stat(&cx(), None, "a.txt")
            .await
            .unwrap()
            .unwrap()
            .modified_at;
        fs.read(&cx(), None, "a.txt", None).await.unwrap();
        let after = fs
            .stat(&cx(), None, "a.txt")
            .await
            .unwrap()
            .unwrap()
            .modified_at;
        assert_eq!(before, after);
    }
}
