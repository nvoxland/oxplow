//! Workspace-files surface — list/read/write/create/rename/delete
//! files inside the project root, with path-traversal protection.
//!
//! Direct port of `src/git/workspace-files.ts`. Path resolution
//! always happens through `resolve_workspace_path`, which rejects
//! anything that escapes the project root after canonicalization.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use specta::Type;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum GitFileStatus {
    Modified,
    Added,
    Deleted,
    Renamed,
    Untracked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceEntryKind {
    File,
    Directory,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct WorkspaceEntry {
    pub name: String,
    pub path: String,
    pub kind: WorkspaceEntryKind,
    pub git_status: Option<GitFileStatus>,
    pub has_changes: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct WorkspaceFile {
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct WorkspaceIndexedFile {
    pub path: String,
    pub git_status: Option<GitFileStatus>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct WorkspaceStatusSummary {
    pub modified: u32,
    pub added: u32,
    pub deleted: u32,
    pub renamed: u32,
    pub untracked: u32,
    pub total: u32,
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

/// List the immediate children of `root_dir + relative_path`,
/// excluding `.git/`. Directories sort before files; otherwise
/// alphabetical. `git_statuses` annotates files (and propagates into
/// `has_changes` for directories that contain changed descendants).
pub fn list_workspace_entries(
    root_dir: &Path,
    relative_path: &str,
    git_statuses: &HashMap<String, GitFileStatus>,
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
        let git_status = if matches!(kind, WorkspaceEntryKind::File) {
            git_statuses.get(&path).copied()
        } else {
            None
        };
        let has_changes = match kind {
            WorkspaceEntryKind::Directory => has_descendant_changes(&path, git_statuses),
            WorkspaceEntryKind::File => git_status.is_some(),
        };
        entries.push(WorkspaceEntry {
            name,
            path,
            kind,
            git_status,
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
    git_statuses: &HashMap<String, GitFileStatus>,
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
            files.extend(list_workspace_files(root_dir, git_statuses, &path, ignore)?);
        } else {
            files.push(WorkspaceIndexedFile {
                path: path.clone(),
                git_status: git_statuses.get(&path).copied(),
            });
        }
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
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

pub fn summarize_git_statuses(
    git_statuses: &HashMap<String, GitFileStatus>,
) -> WorkspaceStatusSummary {
    let mut s = WorkspaceStatusSummary::default();
    for status in git_statuses.values() {
        match status {
            GitFileStatus::Modified => s.modified += 1,
            GitFileStatus::Added => s.added += 1,
            GitFileStatus::Deleted => s.deleted += 1,
            GitFileStatus::Renamed => s.renamed += 1,
            GitFileStatus::Untracked => s.untracked += 1,
        }
        s.total += 1;
    }
    s
}

fn has_descendant_changes(path: &str, git_statuses: &HashMap<String, GitFileStatus>) -> bool {
    let prefix = format!("{path}/");
    git_statuses
        .keys()
        .any(|p| p == path || p.starts_with(&prefix))
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
    let root = root_dir.to_path_buf();
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
        git2::Repository::init(dir.path()).unwrap();
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
    fn summarize_counts_each_status() {
        let mut statuses = HashMap::new();
        statuses.insert("a".into(), GitFileStatus::Modified);
        statuses.insert("b".into(), GitFileStatus::Modified);
        statuses.insert("c".into(), GitFileStatus::Added);
        statuses.insert("d".into(), GitFileStatus::Untracked);
        let s = summarize_git_statuses(&statuses);
        assert_eq!(s.modified, 2);
        assert_eq!(s.added, 1);
        assert_eq!(s.untracked, 1);
        assert_eq!(s.total, 4);
    }

    #[test]
    fn directory_with_changed_descendant_has_changes_flag() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("a")).unwrap();
        std::fs::write(dir.path().join("a/b.txt"), "").unwrap();
        let mut statuses = HashMap::new();
        statuses.insert("a/b.txt".into(), GitFileStatus::Modified);
        let entries = list_workspace_entries(dir.path(), "", &statuses).unwrap();
        let a = entries.iter().find(|e| e.name == "a").unwrap();
        assert!(a.has_changes);
    }
}
