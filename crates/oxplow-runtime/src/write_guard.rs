//! The write guard: the two places no agent writes with a tool. A row a
//! store keeps as a file (a fill's `files:`; an oxwiki page) is written by its
//! extension's command from a worktree (only the primary checkout's folder is
//! followed), and no thread changes another stream's
//! worktree (the primary checkout included). Within its own worktree any
//! thread may write — the stream's threads share it, and who works there
//! at once is the person's call (`.context/agent-model.md` "Isolation").
//! The answer is core's; the agent's harness renders it.

use std::path::{Path, PathBuf};

/// Where a store keeps its rows as files (a fill's `files:`): a file at
/// `<folder>/<id><suffix>` is the row of that id in `store` of `model`,
/// kept by `extension`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordFolder {
    /// Relative to the project root (`.oxplow/oxwiki`).
    pub folder: String,
    pub suffix: String,
    pub store: String,
    pub model: String,
    pub extension: String,
}

/// Why an agent may not write `raw_path` itself, when it is a row file of a
/// store in the thread's own worktree and that worktree is not the primary
/// checkout (`<worktree>/<folder>/<id><suffix>` of a [`RecordFolder`]).
/// Only the primary checkout's folder is watched (`.oxplow` is git-ignored,
/// so a worktree's copy is never committed or followed): a page written there
/// would be silently lost. The store's extension's command writes it, to the
/// primary checkout, from anywhere. In the primary checkout itself a direct
/// write is fine: the store's collector converges it.
pub fn record_file_reason(
    raw_path: Option<&str>,
    worktree_root: Option<&Path>,
    project_dir: Option<&Path>,
    folders: &[RecordFolder],
) -> Option<String> {
    let (raw, worktree_root, project_dir) = (raw_path?, worktree_root?, project_dir?);
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    if canon(worktree_root) == canon(project_dir) {
        return None;
    }
    let path = Path::new(raw);
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        worktree_root.join(path)
    };
    folders.iter().find_map(|f| {
        let name_fits = f.suffix.is_empty()
            || abs
                .file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with(f.suffix.as_str()));
        (name_fits && is_inside(&abs, &worktree_root.join(f.folder.trim_end_matches('/')))).then(
            || {
                format!(
                    "`{}` is a row of store `{}` (`{}`) in this worktree's `{}`, which isn't \
                     watched, so the page would be lost: write it with the extension `{}`'s \
                     command (mcp__oxplow__list_commands lists the `{}.*` commands; its skill \
                     says which), which writes to the primary checkout from anywhere.",
                    abs.display(),
                    f.store,
                    f.model,
                    f.folder,
                    f.extension,
                    f.extension
                )
            },
        )
    })
}

/// Why an agent may not write `path` (absolute, already normalized), when
/// it falls in another stream's worktree: one of `other_roots`, and not
/// the thread's own `worktree_root` (a worktree stream's tree may sit
/// inside the primary checkout's: the deepest tree holding the path owns it).
pub fn foreign_worktree_reason(
    path: &str,
    worktree_root: &Path,
    other_roots: &[PathBuf],
) -> Option<String> {
    let p = Path::new(path);
    if !p.is_absolute() {
        return None;
    }
    // A stream's tree can sit inside another's (a worktree adopted from
    // `<repo>/.claude/worktrees/<name>`): a path belongs to the deepest
    // root that holds it.
    let root = other_roots
        .iter()
        .filter(|root| p.starts_with(root))
        .max_by_key(|root| root.components().count())?;
    if p.starts_with(worktree_root)
        && worktree_root.components().count() >= root.components().count()
    {
        return None;
    }
    Some(format!(
        "path `{path}` is in another stream's worktree (`{}`). A thread may edit only \
         its own stream's worktree; do this from that stream's thread, or record it \
         as a note on the current task.",
        root.display()
    ))
}

fn is_inside(path: &Path, root: &Path) -> bool {
    let path_canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let root_canon = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    path_canon.starts_with(&root_canon)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oxwiki() -> Vec<RecordFolder> {
        vec![RecordFolder {
            folder: ".oxplow/oxwiki".into(),
            suffix: ".md".into(),
            store: "oxwiki".into(),
            model: "oxplow/knowledge_record".into(),
            extension: "oxwiki".into(),
        }]
    }

    #[test]
    fn a_stores_row_file_in_a_worktree_is_written_by_its_extensions_command() {
        let primary = tempfile::tempdir().unwrap();
        let worktree = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(worktree.path().join(".oxplow/oxwiki")).unwrap();
        let target = worktree.path().join(".oxplow/oxwiki/captured.md");
        std::fs::write(&target, "").unwrap();
        let reason = record_file_reason(
            target.to_str(),
            Some(worktree.path()),
            Some(primary.path()),
            &oxwiki(),
        )
        .expect("refused");
        assert!(
            reason.contains("store `oxwiki`") && reason.contains("`oxwiki.*` commands"),
            "{reason}"
        );
        // The primary writes it itself; other files and non-rows are free.
        assert!(record_file_reason(
            target.to_str(),
            Some(primary.path()),
            Some(primary.path()),
            &oxwiki()
        )
        .is_none());
        let other = worktree.path().join(".oxplow/oxwiki/notes.txt");
        for p in [Some("src/a.rs"), other.to_str()] {
            assert!(
                record_file_reason(p, Some(worktree.path()), Some(primary.path()), &oxwiki())
                    .is_none()
            );
        }
        assert!(record_file_reason(
            target.to_str(),
            Some(worktree.path()),
            Some(primary.path()),
            &[]
        )
        .is_none());
    }

    /// Another stream's tree is refused, naming it; the thread's own tree
    /// is not, even where it sits inside another's.
    #[test]
    fn another_streams_worktree_is_refused_naming_it() {
        let others = [PathBuf::from("/proj")];
        let reason = foreign_worktree_reason("/proj/src/a.rs", Path::new("/proj/wt"), &others)
            .expect("refused");
        assert!(
            reason.contains("another stream's worktree (`/proj`)"),
            "{reason}"
        );
        assert!(foreign_worktree_reason("/proj/wt/a.rs", Path::new("/proj/wt"), &others).is_none());
        assert!(foreign_worktree_reason("/tmp/x", Path::new("/proj/wt"), &others).is_none());
    }
}
