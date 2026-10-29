//! `oxplow:local`: the stream's worktree as files (`.context/filesystems.md`).
//! Its revision is the vocabulary of `oxplow_domain::vcs::Revision` and
//! stays private to it: absent or `working` is the files on disk,
//! `snap:<id>` a local-history snapshot, `git:<rev>` a VCS revision.
//!
//! It delegates: reads to [`Trees::read_at`] and [`Trees::files_at`], the
//! working tree's listing and writes to [`WorkspaceFiles`]. `Trees` stays
//! the engine for diffs and the code-quality corpus; this is the funnel a
//! file's content is read through. Only a ranged read of the working tree
//! and its `stat` touch the disk themselves: a range of a big file must
//! not read all of it.

use std::collections::BTreeSet;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use oxplow_config::OxplowConfig;
use oxplow_domain::filesystem::{
    ByteRange, FileSystem, FsCx, FsEntry, FsError, FsKind, FsStat, LOCAL, WRITABLE,
};
use oxplow_domain::language_id::language_id_for_path;
use oxplow_domain::vcs::Revision;

use crate::trees::{relative, Trees};
use crate::workspace_files::{WorkspaceError, WorkspaceFiles};
use crate::worktrees::WorktreeRouter;

/// How much of a file `stat` looks at to tell text from binary.
const SNIFF: usize = 8 * 1024;

pub struct LocalFs {
    trees: Arc<Trees>,
    router: Arc<WorktreeRouter>,
    files: Arc<WorkspaceFiles>,
    config: Arc<RwLock<OxplowConfig>>,
}

impl LocalFs {
    pub fn new(
        trees: Arc<Trees>,
        router: Arc<WorktreeRouter>,
        files: Arc<WorkspaceFiles>,
        config: Arc<RwLock<OxplowConfig>>,
    ) -> Self {
        Self {
            trees,
            router,
            files,
            config,
        }
    }

    /// The tree a call reads: the stream's worktree, the primary checkout
    /// for none (as every read of a stream's files has).
    async fn root(&self, cx: &FsCx) -> PathBuf {
        self.router
            .resolve(cx.stream.map(|s| s.to_string()).as_deref())
            .await
            .into_local_path()
    }
}

fn revision(rev: Option<&str>) -> Result<Revision, FsError> {
    match rev {
        None => Ok(Revision::Working),
        Some(rev) => rev.parse().map_err(FsError::Invalid),
    }
}

/// `parent` as a folder of the workspace: `""` is its root.
fn folder(parent: &str) -> Result<(), FsError> {
    if parent.is_empty() || relative(parent).is_ok() {
        Ok(())
    } else {
        Err(FsError::Invalid(format!(
            "`{parent}` isn't a folder inside the workspace"
        )))
    }
}

fn workspace_error(e: WorkspaceError) -> FsError {
    match e {
        WorkspaceError::PathEscape | WorkspaceError::WholeWorkspace => {
            FsError::Invalid(e.to_string())
        }
        other => FsError::Storage(other.to_string()),
    }
}

async fn blocking<R: Send + 'static>(f: impl FnOnce() -> R + Send + 'static) -> Result<R, FsError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| FsError::Storage(format!("file read didn't finish: {e}")))
}

fn is_text(head: &[u8]) -> bool {
    !head[..head.len().min(SNIFF)].contains(&0)
}

fn file_stat(path: &str, size: u64, modified_at: Option<i64>, head: &[u8]) -> FsStat {
    FsStat {
        kind: FsKind::File,
        size: Some(size),
        modified_at,
        is_text: Some(is_text(head)),
        language: language_id_for_path(path).map(str::to_string),
    }
}

fn dir_stat(modified_at: Option<i64>) -> FsStat {
    FsStat {
        kind: FsKind::Dir,
        size: None,
        modified_at,
        is_text: None,
        language: None,
    }
}

/// `path` on disk under `root`: its stat, or `None` when it isn't there.
pub(super) fn stat_on_disk(root: &Path, path: &str) -> std::io::Result<Option<FsStat>> {
    let abs = root.join(path);
    let meta = match std::fs::metadata(&abs) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_millis()).ok());
    if meta.is_dir() {
        return Ok(Some(dir_stat(modified)));
    }
    let mut head = Vec::new();
    std::fs::File::open(&abs)?
        .take(SNIFF as u64)
        .read_to_end(&mut head)?;
    Ok(Some(file_stat(path, meta.len(), modified, &head)))
}

/// `range` of the file at `root`/`path`, without reading the rest.
pub(super) fn read_range_on_disk(
    root: &Path,
    path: &str,
    range: ByteRange,
) -> std::io::Result<Option<Vec<u8>>> {
    let abs = root.join(path);
    if !abs.is_file() {
        return Ok(None);
    }
    let mut file = std::fs::File::open(abs)?;
    file.seek(SeekFrom::Start(range.offset))?;
    let mut out = Vec::new();
    file.take(range.len).read_to_end(&mut out)?;
    Ok(Some(out))
}

#[async_trait]
impl FileSystem for LocalFs {
    fn id(&self) -> &str {
        LOCAL
    }

    fn features(&self) -> &[&'static str] {
        &[WRITABLE]
    }

    async fn stat(
        &self,
        cx: &FsCx,
        rev: Option<&str>,
        path: &str,
    ) -> Result<Option<FsStat>, FsError> {
        let path = relative(path)?;
        let rev = revision(rev)?;
        let root = self.root(cx).await;
        if rev == Revision::Working {
            let path = path.to_string();
            return blocking(move || stat_on_disk(&root, &path))
                .await?
                .map_err(|e| FsError::Storage(e.to_string()));
        }
        if let Some(bytes) = self.trees.read_at(&root, &rev, path).await? {
            return Ok(Some(file_stat(path, bytes.len() as u64, None, &bytes)));
        }
        // Not a file there: a folder when files lie under it.
        let under = format!("{path}/");
        let files = self.trees.files_at(&root, &rev).await?;
        Ok(files
            .iter()
            .any(|f| f.starts_with(&under))
            .then(|| dir_stat(None)))
    }

    async fn read(
        &self,
        cx: &FsCx,
        rev: Option<&str>,
        path: &str,
        range: Option<ByteRange>,
    ) -> Result<Option<Vec<u8>>, FsError> {
        let path = relative(path)?;
        let rev = revision(rev)?;
        let root = self.root(cx).await;
        if let (Revision::Working, Some(range)) = (&rev, range) {
            let path = path.to_string();
            return blocking(move || read_range_on_disk(&root, &path, range))
                .await?
                .map_err(|e| FsError::Storage(e.to_string()));
        }
        let bytes = self.trees.read_at(&root, &rev, path).await?;
        Ok(match (bytes, range) {
            (Some(bytes), Some(range)) => Some(range.slice(&bytes)),
            (bytes, _) => bytes,
        })
    }

    async fn list(
        &self,
        cx: &FsCx,
        rev: Option<&str>,
        parent: Option<&str>,
    ) -> Result<Vec<FsEntry>, FsError> {
        if let Some(parent) = parent {
            folder(parent)?;
        }
        let rev = revision(rev)?;
        if rev == Revision::Working {
            let generated = self
                .config
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .generated
                .clone();
            let stream = cx.stream.map(|s| s.to_string());
            let paths = self
                .files
                .list_paths(stream.as_deref(), &generated, parent.map(str::to_string))
                .await
                .map_err(workspace_error)?;
            return Ok(paths
                .into_iter()
                .map(|p| FsEntry {
                    path: p.path,
                    kind: if p.is_dir { FsKind::Dir } else { FsKind::File },
                    status: p.status,
                })
                .collect());
        }
        // A tree that lists only files: the folders are the ones they lie in.
        let files = self.trees.files_at(&self.root(cx).await, &rev).await?;
        let mut folders = BTreeSet::new();
        let mut entries = Vec::new();
        for file in files {
            let rest = match parent {
                Some("") | None => file.as_str(),
                Some(parent) => match file.strip_prefix(&format!("{parent}/")) {
                    Some(rest) => rest,
                    None => continue,
                },
            };
            let base = file.len() - rest.len();
            if parent.is_none() {
                for (i, _) in rest.match_indices('/') {
                    folders.insert(file[..base + i].to_string());
                }
            } else if let Some((first, _)) = rest.split_once('/') {
                folders.insert(file[..base + first.len()].to_string());
                continue;
            }
            entries.push(FsEntry {
                path: file,
                kind: FsKind::File,
                status: None,
            });
        }
        entries.extend(folders.into_iter().map(|path| FsEntry {
            path,
            kind: FsKind::Dir,
            status: None,
        }));
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }

    async fn write(&self, cx: &FsCx, path: &str, content: &[u8]) -> Result<(), FsError> {
        if path.is_empty()
            || Path::new(path)
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err(FsError::Invalid(format!(
                "`{path}` isn't a path inside the workspace"
            )));
        }
        let text = std::str::from_utf8(content)
            .map_err(|_| FsError::Invalid(format!("`{path}`: only text can be written")))?;
        let stream = cx.stream.map(|s| s.to_string());
        self.files
            .write(stream.as_deref(), path.to_string(), text.to_string())
            .await
            .map(|_| ())
            .map_err(workspace_error)
    }
}
