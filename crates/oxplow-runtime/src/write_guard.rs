//! Write guard for read-only threads: why a thread that isn't the
//! stream's writer may not make a call that would mutate the shared
//! worktree. The answer is core's; the agent's harness renders it.

use std::path::Path;

use oxplow_domain::Thread;

/// Why an agent may not write `raw_path` itself, when it is a wiki page
/// (`<project>/.oxplow/wiki/…`): pages are written with the
/// `oxplow.knowledge.write_page` command — validated, linked and audited, the
/// file following — whatever the thread (P5.C3). A person's hand edit
/// still converges through the wiki watcher.
pub fn wiki_page_reason(raw_path: Option<&str>, project_dir: Option<&Path>) -> Option<String> {
    let (raw, project_dir) = (raw_path?, project_dir?);
    let path = Path::new(raw);
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_dir.join(path)
    };
    is_inside(&abs, &project_dir.join(".oxplow").join("wiki")).then(|| {
        format!(
            "`{}` is a wiki page: write it with mcp__oxplow__run_command → \
             `oxplow.knowledge.write_page {{ slug, body, verified_refs, removed_refs }}` \
             (it writes the file); `oxplow.knowledge.delete_page {{ slug }}` deletes one.",
            abs.display()
        )
    })
}

/// The write guard's reason, if `thread` may not write `raw_path` (absolute
/// or project-relative; `None` when the call names no path). The core
/// shared by the Claude hook response and [`crate::policy::decide_tool`].
/// `None` for a writer thread or a path outside both the project and
/// `.oxplow/`.
pub fn read_only_reason(
    thread: &Thread,
    raw_path: Option<&str>,
    project_dir: Option<&Path>,
) -> Option<String> {
    if thread.status.is_writer() {
        return None;
    }
    if let (Some(project_dir), Some(raw)) = (project_dir, raw_path) {
        let path = Path::new(raw);
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            project_dir.join(path)
        };
        let oxplow_dir = project_dir.join(".oxplow");
        let inside_project = is_inside(&abs, project_dir);
        let inside_oxplow = is_inside(&abs, &oxplow_dir);
        if !inside_project && !inside_oxplow {
            return None;
        }
        return Some(format!(
            "path `{}` is inside the shared worktree and this thread is read-only — \
             only the stream's writer thread may mutate the worktree. \
             Record the change as a note on the current task via mcp__oxplow tools (or stop this turn). \
             To edit here, make this thread the writer (`oxplow.thread.promote` through mcp__oxplow__run_command) — it takes the worktree from the current writer, so do it only when this thread's work should go first.",
            abs.display()
        ));
    }
    Some(
        "This thread is read-only — only the stream's writer thread may mutate the worktree. \
         Record the change as a note on the current task via mcp__oxplow tools (or stop this turn). \
         To edit here, make this thread the writer (`oxplow.thread.promote` through mcp__oxplow__run_command) — it takes the worktree from the current writer, so do it only when this thread's work should go first."
            .into(),
    )
}

fn is_inside(path: &Path, root: &Path) -> bool {
    let path_canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let root_canon = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    path_canon.starts_with(&root_canon)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::{StreamId, ThreadId, ThreadStatus, Timestamp};

    fn read_only_thread() -> Thread {
        Thread {
            id: ThreadId::new(2),
            stream_id: StreamId::new(1),
            title: "explore".into(),
            status: ThreadStatus::Queued,
            sort_index: 0,
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
            archived_at: None,
        }
    }

    #[test]
    fn a_writer_is_never_read_only() {
        let t = Thread {
            status: ThreadStatus::Active,
            ..read_only_thread()
        };
        assert!(read_only_reason(&t, Some("src/a.rs"), Some(Path::new("/p"))).is_none());
    }

    #[test]
    fn a_closed_thread_is_read_only_like_a_queued_one() {
        let mut t = read_only_thread();
        t.status = ThreadStatus::Closed;
        let reason = read_only_reason(&t, None, None).expect("refused");
        assert!(reason.contains("read-only"), "{reason}");
    }

    #[test]
    fn outside_the_project_is_allowed_inside_is_refused_naming_the_path() {
        let t = read_only_thread();
        let project = tempfile::tempdir().unwrap();
        assert!(
            read_only_reason(&t, Some("/tmp/somewhere/else.txt"), Some(project.path())).is_none()
        );
        let target = project.path().join("src/foo.rs");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "").unwrap();
        let reason = read_only_reason(&t, target.to_str(), Some(project.path())).expect("refused");
        assert!(reason.contains("inside the shared worktree"), "{reason}");
        std::fs::create_dir_all(project.path().join(".oxplow/runtime")).unwrap();
        let state = project.path().join(".oxplow/runtime/local.sqlite");
        std::fs::write(&state, "").unwrap();
        assert!(read_only_reason(&t, state.to_str(), Some(project.path())).is_some());
    }

    #[test]
    fn a_wiki_page_is_written_by_command() {
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(project.path().join(".oxplow/wiki")).unwrap();
        let target = project.path().join(".oxplow/wiki/captured.md");
        std::fs::write(&target, "").unwrap();
        let reason = wiki_page_reason(target.to_str(), Some(project.path())).expect("refused");
        assert!(reason.contains("oxplow.knowledge.write_page"), "{reason}");
        assert!(wiki_page_reason(Some("src/a.rs"), Some(project.path())).is_none());
    }
}
