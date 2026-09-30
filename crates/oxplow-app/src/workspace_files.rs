//! A workspace's files (`.context/vcs.md`): list, read, write, create,
//! rename and delete under a stream's workspace, with path-traversal
//! protection, annotated with the VCS's status. Plain file I/O — no VCS
//! call beyond `status` — so it lives beside the router, not in a
//! provider. A write announces `WorkspaceChanged` for its stream.
//!
//! Every path resolves through `resolve_workspace_path`, which rejects
//! anything that escapes the root.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

use oxplow_domain::vcs::{FileStatus, Vcs};
use oxplow_domain::StreamId;

use crate::events::{EventBus, OxplowEvent, WorkspaceChangeKind};
use crate::worktrees::WorktreeRouter;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceEntryKind {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceEntry {
    pub name: String,
    pub path: String,
    pub kind: WorkspaceEntryKind,
    pub status: Option<FileStatus>,
    pub has_changes: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct WorkspaceFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceIndexedFile {
    pub path: String,
    pub status: Option<FileStatus>,
}

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("path resolves outside workspace")]
    PathEscape,
    #[error("path does not exist")]
    NotFound,
    #[error("path already exists")]
    AlreadyExists,
}

/// A stream's files. Held in `Services` as `workspace_files`.
pub struct WorkspaceFiles {
    router: Arc<WorktreeRouter>,
    vcs: Arc<dyn Vcs>,
    events: EventBus,
}

impl WorkspaceFiles {
    pub fn new(router: Arc<WorktreeRouter>, vcs: Arc<dyn Vcs>, events: EventBus) -> Self {
        Self {
            router,
            vcs,
            events,
        }
    }

    /// Path → status for the workspace; empty when it isn't under
    /// version control.
    async fn statuses(&self, root: &Path) -> HashMap<String, FileStatus> {
        self.vcs
            .status(root)
            .await
            .map(|s| s.entries.into_iter().map(|e| (e.path, e.status)).collect())
            .unwrap_or_default()
    }

    fn announce(&self, stream_id: Option<&str>) {
        if let Some(id) = stream_id.and_then(StreamId::try_from_str) {
            self.events.emit(OxplowEvent::WorkspaceChanged {
                stream_id: id,
                change_kind: WorkspaceChangeKind::Updated,
                path: String::new(),
            });
        }
    }

    async fn blocking<R: Send + 'static>(
        f: impl FnOnce() -> Result<R, WorkspaceError> + Send + 'static,
    ) -> Result<R, WorkspaceError> {
        tokio::task::spawn_blocking(f)
            .await
            .map_err(|e| WorkspaceError::Io(std::io::Error::other(e.to_string())))?
    }

    pub async fn list_entries(
        &self,
        stream_id: Option<&str>,
        relative_path: String,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        let root = self.router.resolve(stream_id).await;
        let statuses = self.statuses(&root).await;
        Self::blocking(move || list_workspace_entries(&root, &relative_path, &statuses)).await
    }

    /// Every file, pruned by `filter` (the project's `generated:`
    /// exclusions), which also bounds the walk.
    pub async fn list_files(
        &self,
        stream_id: Option<&str>,
        filter: oxplow_fs_watch::WorkspaceFilter,
    ) -> Result<Vec<WorkspaceIndexedFile>, WorkspaceError> {
        let root = self.router.resolve(stream_id).await;
        let statuses = self.statuses(&root).await;
        Self::blocking(move || {
            list_workspace_files(&root, &statuses, "", &|path| {
                filter.ignore(Path::new(path), false)
            })
        })
        .await
    }

    /// Lines containing `query` across the files `filter` keeps
    /// ([`search_workspace_text`]).
    pub async fn search_text(
        &self,
        stream_id: Option<&str>,
        filter: oxplow_fs_watch::WorkspaceFilter,
        query: String,
        limit: usize,
    ) -> Result<Vec<TextSearchHit>, WorkspaceError> {
        let root = self.router.resolve(stream_id).await;
        Self::blocking(move || {
            search_workspace_text(&root, &query, limit, &|path| {
                filter.ignore(Path::new(path), false)
            })
        })
        .await
    }

    pub async fn read(
        &self,
        stream_id: Option<&str>,
        relative_path: String,
    ) -> Result<WorkspaceFile, WorkspaceError> {
        let root = self.router.resolve(stream_id).await;
        Self::blocking(move || read_workspace_file(&root, &relative_path)).await
    }

    pub async fn write(
        &self,
        stream_id: Option<&str>,
        relative_path: String,
        content: String,
    ) -> Result<WorkspaceFile, WorkspaceError> {
        let root = self.router.resolve(stream_id).await;
        let out =
            Self::blocking(move || write_workspace_file(&root, &relative_path, &content)).await?;
        self.announce(stream_id);
        Ok(out)
    }

    pub async fn create_file(
        &self,
        stream_id: Option<&str>,
        relative_path: String,
        content: String,
    ) -> Result<WorkspaceFile, WorkspaceError> {
        let root = self.router.resolve(stream_id).await;
        let out =
            Self::blocking(move || create_workspace_file(&root, &relative_path, &content)).await?;
        self.announce(stream_id);
        Ok(out)
    }

    pub async fn create_directory(
        &self,
        stream_id: Option<&str>,
        relative_path: String,
    ) -> Result<String, WorkspaceError> {
        let root = self.router.resolve(stream_id).await;
        Self::blocking(move || create_workspace_directory(&root, &relative_path)).await
    }

    pub async fn rename(
        &self,
        stream_id: Option<&str>,
        from_path: String,
        to_path: String,
    ) -> Result<(String, String), WorkspaceError> {
        let root = self.router.resolve(stream_id).await;
        let out =
            Self::blocking(move || rename_workspace_path(&root, &from_path, &to_path)).await?;
        self.announce(stream_id);
        Ok(out)
    }

    pub async fn delete(
        &self,
        stream_id: Option<&str>,
        relative_path: String,
    ) -> Result<String, WorkspaceError> {
        let root = self.router.resolve(stream_id).await;
        let out = Self::blocking(move || delete_workspace_path(&root, &relative_path)).await?;
        self.announce(stream_id);
        Ok(out)
    }
}

/// List the immediate children of `root_dir + relative_path`,
/// excluding `.git/`. Directories sort before files; otherwise
/// alphabetical. `statuses` annotates files (and propagates into
/// `has_changes` for directories that contain changed descendants).
pub fn list_workspace_entries(
    root_dir: &Path,
    relative_path: &str,
    statuses: &HashMap<String, FileStatus>,
) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
    let dir = resolve_workspace_path(root_dir, relative_path)?;
    let mut entries: Vec<WorkspaceEntry> = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        let kind = if entry.file_type()?.is_dir() {
            WorkspaceEntryKind::Directory
        } else {
            WorkspaceEntryKind::File
        };
        let path = normalize_relative_path(relative_path, &name);
        let status = if matches!(kind, WorkspaceEntryKind::File) {
            statuses.get(&path).copied()
        } else {
            None
        };
        let has_changes = match kind {
            WorkspaceEntryKind::Directory => has_descendant_changes(&path, statuses),
            WorkspaceEntryKind::File => status.is_some(),
        };
        entries.push(WorkspaceEntry {
            name,
            path,
            kind,
            status,
            has_changes,
        });
    }
    entries.sort_by(|a, b| match (a.kind, b.kind) {
        (WorkspaceEntryKind::Directory, WorkspaceEntryKind::File) => std::cmp::Ordering::Less,
        (WorkspaceEntryKind::File, WorkspaceEntryKind::Directory) => std::cmp::Ordering::Greater,
        _ => a.name.cmp(&b.name),
    });
    Ok(entries)
}

/// Recursive flatten — every file under `root_dir`, sorted by path.
/// `ignore` is consulted with each entry's workspace-relative path;
/// ignored directories are pruned (not descended), so the caller's
/// `generated:` exclusions also bound the walk's cost — a node_modules
/// tree is hundreds of thousands of entries the quick-open index has
/// no use for.
///
/// Exclusion is driven solely by `ignore` (the `generated:` config
/// list), matching fs-watch/snapshots. `.gitignore` is deliberately
/// NOT consulted: a path being absent from git doesn't mean the user
/// doesn't want to find it. The single source of truth for "don't
/// index this" is the `generated:` list.
pub fn list_workspace_files(
    root_dir: &Path,
    statuses: &HashMap<String, FileStatus>,
    relative_path: &str,
    ignore: &dyn Fn(&str) -> bool,
) -> Result<Vec<WorkspaceIndexedFile>, WorkspaceError> {
    let dir = resolve_workspace_path(root_dir, relative_path)?;
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        let path = normalize_relative_path(relative_path, &name);
        if ignore(&path) {
            continue;
        }
        if entry.file_type()?.is_dir() {
            files.extend(list_workspace_files(root_dir, statuses, &path, ignore)?);
        } else {
            files.push(WorkspaceIndexedFile {
                path: path.clone(),
                status: statuses.get(&path).copied(),
            });
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// A line of a workspace file that matched a text search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct TextSearchHit {
    pub path: String,
    /// 1-based.
    pub line: u32,
    /// The line, cut at 400 bytes.
    pub snippet: String,
}

/// Every line of the workspace's files containing `query` (a fixed,
/// case-sensitive string, trimmed), at most `limit`, over the files
/// `ignore` keeps — the same walk as quick-open. Binary files (a NUL in
/// the first 8 KiB) are skipped.
pub fn search_workspace_text(
    root_dir: &Path,
    query: &str,
    limit: usize,
    ignore: &dyn Fn(&str) -> bool,
) -> Result<Vec<TextSearchHit>, WorkspaceError> {
    const SNIPPET_BYTES: usize = 400;
    let query = query.trim();
    let mut hits = Vec::new();
    if query.is_empty() || limit == 0 {
        return Ok(hits);
    }
    for file in list_workspace_files(root_dir, &HashMap::new(), "", ignore)? {
        let Ok(bytes) = std::fs::read(root_dir.join(&file.path)) else {
            continue;
        };
        if bytes[..bytes.len().min(8192)].contains(&0) {
            continue;
        }
        let text = String::from_utf8_lossy(&bytes);
        for (i, line) in text.lines().enumerate() {
            if !line.contains(query) {
                continue;
            }
            let snippet = if line.len() > SNIPPET_BYTES {
                let mut end = SNIPPET_BYTES;
                while !line.is_char_boundary(end) {
                    end -= 1;
                }
                format!("{}…", &line[..end])
            } else {
                line.to_string()
            };
            hits.push(TextSearchHit {
                path: file.path.clone(),
                line: i as u32 + 1,
                snippet,
            });
            if hits.len() >= limit {
                return Ok(hits);
            }
        }
    }
    Ok(hits)
}

pub fn read_workspace_file(
    root_dir: &Path,
    relative_path: &str,
) -> Result<WorkspaceFile, WorkspaceError> {
    let path = clean_relative_path(relative_path);
    let abs = resolve_workspace_path(root_dir, &path)?;
    let content = std::fs::read_to_string(abs)?;
    Ok(WorkspaceFile { path, content })
}

pub fn write_workspace_file(
    root_dir: &Path,
    relative_path: &str,
    content: &str,
) -> Result<WorkspaceFile, WorkspaceError> {
    let path = clean_relative_path(relative_path);
    let abs = resolve_workspace_path(root_dir, &path)?;
    std::fs::write(abs, content.as_bytes())?;
    Ok(WorkspaceFile {
        path,
        content: content.to_string(),
    })
}

pub fn create_workspace_file(
    root_dir: &Path,
    relative_path: &str,
    content: &str,
) -> Result<WorkspaceFile, WorkspaceError> {
    let path = clean_relative_path(relative_path);
    let abs = resolve_workspace_path(root_dir, &path)?;
    if abs.exists() {
        return Err(WorkspaceError::AlreadyExists);
    }
    if let Some(parent) = abs.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&abs, content.as_bytes())?;
    Ok(WorkspaceFile {
        path,
        content: content.to_string(),
    })
}

pub fn create_workspace_directory(
    root_dir: &Path,
    relative_path: &str,
) -> Result<String, WorkspaceError> {
    let path = clean_relative_path(relative_path);
    let abs = resolve_workspace_path(root_dir, &path)?;
    if abs.exists() {
        return Err(WorkspaceError::AlreadyExists);
    }
    std::fs::create_dir_all(abs)?;
    Ok(path)
}

pub fn rename_workspace_path(
    root_dir: &Path,
    from_path: &str,
    to_path: &str,
) -> Result<(String, String), WorkspaceError> {
    let from = clean_relative_path(from_path);
    let to = clean_relative_path(to_path);
    let from_abs = resolve_workspace_path(root_dir, &from)?;
    let to_abs = resolve_workspace_path(root_dir, &to)?;
    if !from_abs.exists() {
        return Err(WorkspaceError::NotFound);
    }
    if to_abs.exists() {
        return Err(WorkspaceError::AlreadyExists);
    }
    if let Some(parent) = to_abs.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&from_abs, &to_abs)?;
    Ok((from, to))
}

pub fn delete_workspace_path(
    root_dir: &Path,
    relative_path: &str,
) -> Result<String, WorkspaceError> {
    let path = clean_relative_path(relative_path);
    let abs = resolve_workspace_path(root_dir, &path)?;
    if !abs.exists() {
        return Err(WorkspaceError::NotFound);
    }
    if abs.is_dir() {
        std::fs::remove_dir_all(abs)?;
    } else {
        std::fs::remove_file(abs)?;
    }
    Ok(path)
}

fn has_descendant_changes(path: &str, statuses: &HashMap<String, FileStatus>) -> bool {
    let prefix = format!("{path}/");
    statuses.keys().any(|p| p == path || p.starts_with(&prefix))
}

fn normalize_relative_path(base: &str, name: &str) -> String {
    if base.is_empty() {
        name.to_string()
    } else {
        format!("{base}/{name}")
    }
}

fn clean_relative_path(relative_path: &str) -> String {
    relative_path.trim_start_matches('/').to_string()
}

/// Resolve `root + relative` and reject anything that escapes the
/// root after canonicalization. The TS version did a string-prefix
/// check; we use the same approach since canonicalize fails on
/// non-existent paths (which is fine for read paths but breaks for
/// create paths).
fn resolve_workspace_path(root_dir: &Path, relative_path: &str) -> Result<PathBuf, WorkspaceError> {
    let clean = clean_relative_path(relative_path);
    // Absolutize the root (lexically — no fs access, unlike
    // `canonicalize`) so the separator-prefix containment check below
    // can't false-positive on a relative root like `.`, where
    // `normalize_path` yields `""` and every child then reads as
    // escaping the root. Production roots are already absolute (the
    // launcher canonicalizes `project_dir`); this is defense in depth.
    let root = std::path::absolute(root_dir).unwrap_or_else(|_| root_dir.to_path_buf());
    let abs = if clean.is_empty() {
        root.clone()
    } else {
        root.join(&clean)
    };
    // String-level check: the resolved path must equal root or live
    // under it (separator-aware). This catches `..` traversal without
    // requiring the path to exist.
    let abs_normalized = normalize_path(&abs);
    let root_normalized = normalize_path(&root);
    if abs_normalized != root_normalized
        && !abs_normalized.starts_with(&format!("{root_normalized}{}", std::path::MAIN_SEPARATOR))
    {
        return Err(WorkspaceError::PathEscape);
    }
    Ok(abs)
}

/// Normalize a path: collapse `.` / `..` segments without requiring
/// the path to exist. Replaces `std::fs::canonicalize` for write
/// paths that don't exist yet.
fn normalize_path(path: &Path) -> String {
    let mut components = Vec::new();
    for c in path.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                components.pop();
            }
            other => components.push(other),
        }
    }
    components
        .iter()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(std::path::MAIN_SEPARATOR_STR)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn text_search_finds_fixed_strings_in_the_files_the_filter_keeps() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "fn a() {}\nlet x = foo(1);\n").unwrap();
        std::fs::write(root.join("b.txt"), "foo( twice foo(\n").unwrap();
        std::fs::write(root.join("bin.dat"), b"foo(\0\x01").unwrap();
        std::fs::create_dir(root.join("gen")).unwrap();
        std::fs::write(root.join("gen/out.rs"), "foo(\n").unwrap();
        let skip_gen = |p: &str| p.starts_with("gen");

        let hits = search_workspace_text(root, "foo(", 200, &skip_gen).unwrap();
        assert_eq!(
            hits,
            vec![
                TextSearchHit {
                    path: "b.txt".into(),
                    line: 1,
                    snippet: "foo( twice foo(".into()
                },
                TextSearchHit {
                    path: "src/a.rs".into(),
                    line: 2,
                    snippet: "let x = foo(1);".into()
                },
            ],
            "binary and filtered files are skipped; one hit per line"
        );
        assert_eq!(
            search_workspace_text(root, "foo(", 1, &skip_gen)
                .unwrap()
                .len(),
            1
        );
        assert!(search_workspace_text(root, "  ", 200, &skip_gen)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn a_long_line_is_cut_on_a_character_boundary() {
        let dir = tempdir().unwrap();
        let line = format!("needle {}", "é".repeat(400));
        std::fs::write(dir.path().join("long.txt"), &line).unwrap();
        let hits = search_workspace_text(dir.path(), "needle", 10, &|_| false).unwrap();
        assert!(hits[0].snippet.ends_with('…'));
        assert!(hits[0].snippet.len() <= 404);
    }

    #[test]
    fn list_entries_sorts_dirs_before_files() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("aaa")).unwrap();
        std::fs::write(dir.path().join("bbb.txt"), "").unwrap();
        std::fs::write(dir.path().join("ccc.txt"), "").unwrap();
        let entries = list_workspace_entries(dir.path(), "", &HashMap::new()).unwrap();
        assert_eq!(entries[0].name, "aaa");
        assert_eq!(entries[0].kind, WorkspaceEntryKind::Directory);
        assert_eq!(entries[1].name, "bbb.txt");
        assert_eq!(entries[2].name, "ccc.txt");
    }

    #[test]
    fn list_entries_skips_dot_git() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join("a.txt"), "").unwrap();
        let entries = list_workspace_entries(dir.path(), "", &HashMap::new()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "a.txt");
    }

    #[test]
    fn write_then_read_round_trips() {
        let dir = tempdir().unwrap();
        write_workspace_file(dir.path(), "hello.txt", "world").unwrap();
        let f = read_workspace_file(dir.path(), "hello.txt").unwrap();
        assert_eq!(f.path, "hello.txt");
        assert_eq!(f.content, "world");
    }

    #[test]
    fn create_file_rejects_existing() {
        let dir = tempdir().unwrap();
        create_workspace_file(dir.path(), "a.txt", "").unwrap();
        let err = create_workspace_file(dir.path(), "a.txt", "").unwrap_err();
        assert!(matches!(err, WorkspaceError::AlreadyExists));
    }

    #[test]
    fn rename_moves_file() {
        let dir = tempdir().unwrap();
        write_workspace_file(dir.path(), "a.txt", "x").unwrap();
        rename_workspace_path(dir.path(), "a.txt", "b.txt").unwrap();
        assert!(read_workspace_file(dir.path(), "a.txt").is_err());
        assert_eq!(
            read_workspace_file(dir.path(), "b.txt").unwrap().content,
            "x"
        );
    }

    #[test]
    fn delete_removes_directory_recursively() {
        let dir = tempdir().unwrap();
        create_workspace_directory(dir.path(), "sub").unwrap();
        write_workspace_file(dir.path(), "sub/a.txt", "").unwrap();
        delete_workspace_path(dir.path(), "sub").unwrap();
        assert!(!dir.path().join("sub").exists());
    }

    #[test]
    fn path_escape_is_rejected() {
        let dir = tempdir().unwrap();
        let err = read_workspace_file(dir.path(), "../escape.txt").unwrap_err();
        assert!(matches!(err, WorkspaceError::PathEscape));
    }

    #[test]
    fn relative_root_resolves_children_without_false_escape() {
        // Regression (tsk160): a relative root like `.` made
        // `normalize_path` collapse to "", so every child
        // false-positived as escaping the root and killed the whole
        // listing. The guard now absolutizes the root so containment is
        // computed correctly — children resolve, real traversal still
        // rejects. (The launcher also canonicalizes `project_dir`, so a
        // relative root never reaches here in production; this is the
        // defense-in-depth half.)
        let resolved = resolve_workspace_path(Path::new("."), "app").unwrap();
        assert!(resolved.is_absolute());
        assert!(resolved.ends_with("app"));

        let err = resolve_workspace_path(Path::new("."), "../escape.txt").unwrap_err();
        assert!(matches!(err, WorkspaceError::PathEscape));
    }

    #[test]
    fn list_files_recurses() {
        let dir = tempdir().unwrap();
        create_workspace_directory(dir.path(), "sub").unwrap();
        write_workspace_file(dir.path(), "sub/deep.txt", "").unwrap();
        write_workspace_file(dir.path(), "top.txt", "").unwrap();
        let files = list_workspace_files(dir.path(), &HashMap::new(), "", &|_| false).unwrap();
        let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["sub/deep.txt", "top.txt"]);
    }

    #[test]
    fn list_files_prunes_ignored_directories_and_files() {
        let dir = tempdir().unwrap();
        create_workspace_directory(dir.path(), "node_modules/pkg").unwrap();
        write_workspace_file(dir.path(), "node_modules/pkg/index.js", "").unwrap();
        create_workspace_directory(dir.path(), "src").unwrap();
        write_workspace_file(dir.path(), "src/main.rs", "").unwrap();
        write_workspace_file(dir.path(), "junk.log", "").unwrap();
        let ignore = |path: &str| path.starts_with("node_modules") || path == "junk.log";
        let files = list_workspace_files(dir.path(), &HashMap::new(), "", &ignore).unwrap();
        let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["src/main.rs"]);
    }

    #[test]
    fn list_files_does_not_consult_gitignore() {
        // `.gitignore` is deliberately NOT honored — the `generated:`
        // list (the `ignore` closure) is the single source of truth for
        // exclusions. A gitignored-but-not-generated path stays visible.
        let dir = tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        write_workspace_file(dir.path(), ".gitignore", "dist/\nsecret.log\n").unwrap();
        create_workspace_directory(dir.path(), "dist").unwrap();
        write_workspace_file(dir.path(), "dist/bundle.js", "").unwrap();
        create_workspace_directory(dir.path(), "src").unwrap();
        write_workspace_file(dir.path(), "src/main.rs", "").unwrap();
        write_workspace_file(dir.path(), "secret.log", "").unwrap();
        let files = list_workspace_files(dir.path(), &HashMap::new(), "", &|_| false).unwrap();
        let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![".gitignore", "dist/bundle.js", "secret.log", "src/main.rs"]
        );
    }

    #[test]
    fn directory_with_changed_descendant_has_changes_flag() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("a")).unwrap();
        std::fs::write(dir.path().join("a/b.txt"), "").unwrap();
        let mut statuses = HashMap::new();
        statuses.insert("a/b.txt".into(), FileStatus::Modified);
        let entries = list_workspace_entries(dir.path(), "", &statuses).unwrap();
        let a = entries.iter().find(|e| e.name == "a").unwrap();
        assert!(a.has_changes);
    }
}
