//! The write guard: the two places no agent writes with a tool. An oxwiki
//! page is written by command, and no thread changes another stream's
//! worktree (the primary checkout included). Within its own worktree any
//! thread may write — the stream's threads share it, and who works there
//! at once is the person's call (`.context/agent-model.md` "Isolation").
//! The answer is core's; the agent's harness renders it.

use std::path::{Path, PathBuf};

/// Why an agent may not write `raw_path` itself, when it is an oxwiki page
/// (`<project>/.oxplow/oxwiki/…`): pages are written with the
/// `oxwiki.page.write` command — validated, linked and audited, the file
/// following — whatever the thread. A person's hand edit still
/// converges through oxwiki's watch.
pub fn oxwiki_page_reason(raw_path: Option<&str>, project_dir: Option<&Path>) -> Option<String> {
    let (raw, project_dir) = (raw_path?, project_dir?);
    let path = Path::new(raw);
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_dir.join(path)
    };
    is_inside(
        &abs,
        &project_dir.join(oxplow_domain::knowledge::OXWIKI_DIR),
    )
    .then(|| {
        format!(
            "`{}` is an oxwiki page: write it with mcp__oxplow__run_command → \
             `oxwiki.page.write {{ id, body }}` (it writes the file); \
             `oxwiki.page.delete {{ id }}` deletes one.",
            abs.display()
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

    #[test]
    fn an_oxwiki_page_is_written_by_command() {
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(project.path().join(".oxplow/oxwiki")).unwrap();
        let target = project.path().join(".oxplow/oxwiki/captured.md");
        std::fs::write(&target, "").unwrap();
        let reason = oxwiki_page_reason(target.to_str(), Some(project.path())).expect("refused");
        assert!(reason.contains("oxwiki.page.write"), "{reason}");
        assert!(oxwiki_page_reason(Some("src/a.rs"), Some(project.path())).is_none());
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
