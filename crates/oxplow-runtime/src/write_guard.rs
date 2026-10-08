//! Write guard for read-only threads.
//!
//! Direct port of `src/electron/write-guard.ts`. Returns a
//! Claude-Code-shaped PreToolUse deny body when the calling thread is
//! not the stream's writer and the tool would mutate the shared
//! worktree.

use std::path::Path;
use std::sync::OnceLock;

use serde::Serialize;
use serde_json::Value;
use specta::Type;

use oxplow_domain::Thread;

/// Tool names that mutate the shared worktree. Bash is intentionally
/// excluded; see the TS source for rationale.
pub fn worktree_mutating_tools() -> &'static [&'static str] {
    static SET: OnceLock<[&'static str; 4]> = OnceLock::new();
    SET.get_or_init(|| ["Write", "Edit", "MultiEdit", "NotebookEdit"])
        .as_slice()
}

/// Convenience constant for callers that just want the slice.
pub static WORKTREE_MUTATING_TOOLS: &[&str] = &["Write", "Edit", "MultiEdit", "NotebookEdit"];

/// Body shape that mirrors Claude Code's hook response contract.
#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub struct WriteGuardDeny {
    #[serde(rename = "hookSpecificOutput")]
    pub hook_specific_output: HookSpecificOutput,
}

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub struct HookSpecificOutput {
    #[serde(rename = "hookEventName")]
    pub hook_event_name: &'static str,
    #[serde(rename = "permissionDecision")]
    pub permission_decision: &'static str,
    #[serde(rename = "permissionDecisionReason")]
    pub permission_decision_reason: String,
}

#[derive(Debug, Clone, Default)]
pub struct WriteGuardContext<'a> {
    /// Absolute path to the project root (the shared worktree root).
    pub project_dir: Option<&'a Path>,
    /// Raw `tool_input` JSON from the PreToolUse payload.
    pub tool_input: Option<&'a Value>,
}

/// Returns a deny body when the thread is not the stream's writer and
/// the tool would mutate the shared worktree. Returns `None` to let
/// the call proceed.
///
/// "Writer" is `ThreadStatus::Active`; everything else
/// (`Queued`, `Closed`) is read-only.
pub fn build_write_guard_response(
    thread: Option<&Thread>,
    tool_name: &str,
    context: WriteGuardContext<'_>,
) -> Option<WriteGuardDeny> {
    let thread = thread?;
    if tool_name.is_empty() || tool_name.starts_with("mcp__") {
        return None;
    }
    if !WORKTREE_MUTATING_TOOLS.contains(&tool_name) {
        return None;
    }
    let raw = context.tool_input.and_then(raw_target_path);
    let deny = |reason: String| WriteGuardDeny {
        hook_specific_output: HookSpecificOutput {
            hook_event_name: "PreToolUse",
            permission_decision: "deny",
            permission_decision_reason: reason,
        },
    };
    if let Some(reason) = wiki_page_reason(raw, context.project_dir) {
        return Some(deny(reason));
    }
    if thread.status.is_writer() {
        return None;
    }
    // Without a project dir the path can't be placed: the generic reason.
    let raw = if context.project_dir.is_some() {
        raw
    } else {
        None
    };
    read_only_reason(thread, raw, context.project_dir).map(|reason| WriteGuardDeny {
        hook_specific_output: HookSpecificOutput {
            hook_event_name: "PreToolUse",
            permission_decision: "deny",
            permission_decision_reason: reason,
        },
    })
}

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

/// The target path a Claude-shaped `tool_input` names.
fn raw_target_path(tool_input: &Value) -> Option<&str> {
    tool_input
        .get("file_path")
        .and_then(|v| v.as_str())
        .or_else(|| tool_input.get("notebook_path").and_then(|v| v.as_str()))
        .or_else(|| tool_input.get("path").and_then(|v| v.as_str()))
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
    use serde_json::json;

    fn read_only_thread() -> Thread {
        Thread {
            id: ThreadId::new(2),
            stream_id: StreamId::new(1),
            title: "explore".into(),
            status: ThreadStatus::Queued,
            sort_index: 0,
            pane_target: "talking".into(),
            agent: oxplow_domain::AgentKind::Claude,
            acp_agent: None,
            resume_session_id: String::new(),
            summary: String::new(),
            summary_updated_at: None,
            closed_at: None,
            custom_prompt: None,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
            archived_at: None,
        }
    }

    fn writer_thread() -> Thread {
        Thread {
            status: ThreadStatus::Active,
            ..read_only_thread()
        }
    }

    #[test]
    fn no_thread_means_no_deny() {
        let result = build_write_guard_response(None, "Write", WriteGuardContext::default());
        assert!(result.is_none());
    }

    #[test]
    fn writer_thread_never_denied() {
        let t = writer_thread();
        let result = build_write_guard_response(Some(&t), "Write", WriteGuardContext::default());
        assert!(result.is_none(), "writer thread must be allowed to mutate");
    }

    #[test]
    fn closed_thread_treated_as_read_only() {
        let mut t = read_only_thread();
        t.status = ThreadStatus::Closed;
        let result = build_write_guard_response(Some(&t), "Write", WriteGuardContext::default());
        assert!(result.is_some(), "closed thread must be denied like queued");
    }

    #[test]
    fn mcp_tool_never_denied() {
        let t = read_only_thread();
        let result = build_write_guard_response(
            Some(&t),
            "mcp__oxplow__run_command",
            WriteGuardContext::default(),
        );
        assert!(result.is_none());
    }

    #[test]
    fn non_mutating_tool_never_denied() {
        let t = read_only_thread();
        let result = build_write_guard_response(Some(&t), "Read", WriteGuardContext::default());
        assert!(result.is_none());
    }

    #[test]
    fn write_without_path_context_returns_generic_deny() {
        let t = read_only_thread();
        let result = build_write_guard_response(Some(&t), "Write", WriteGuardContext::default());
        let body = result.expect("deny");
        assert_eq!(body.hook_specific_output.permission_decision, "deny");
        assert!(body
            .hook_specific_output
            .permission_decision_reason
            .contains("read-only"));
    }

    #[test]
    fn write_outside_project_allowed_when_path_known() {
        let t = read_only_thread();
        let project = tempfile::tempdir().unwrap();
        let outside = "/tmp/somewhere/else.txt";
        let input = json!({"file_path": outside});
        let result = build_write_guard_response(
            Some(&t),
            "Write",
            WriteGuardContext {
                project_dir: Some(project.path()),
                tool_input: Some(&input),
            },
        );
        assert!(result.is_none(), "outside project should be allowed");
    }

    #[test]
    fn a_wiki_page_is_written_by_command_whatever_the_thread() {
        let t = read_only_thread();
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(project.path().join(".oxplow/wiki")).unwrap();
        let target = project.path().join(".oxplow/wiki/captured.md");
        std::fs::write(&target, "").unwrap();
        let input = json!({"file_path": target.to_str().unwrap()});
        let result = build_write_guard_response(
            Some(&t),
            "Write",
            WriteGuardContext {
                project_dir: Some(project.path()),
                tool_input: Some(&input),
            },
        );
        let reason = result
            .expect("a wiki page write is refused")
            .hook_specific_output
            .permission_decision_reason;
        assert!(reason.contains("oxplow.knowledge.write_page"), "{reason}");
        let mut writer = t.clone();
        writer.status = ThreadStatus::Active;
        assert!(build_write_guard_response(
            Some(&writer),
            "Write",
            WriteGuardContext {
                project_dir: Some(project.path()),
                tool_input: Some(&input),
            },
        )
        .is_some());
    }

    #[test]
    fn write_inside_project_denied_with_path_in_reason() {
        let t = read_only_thread();
        let project = tempfile::tempdir().unwrap();
        let target = project.path().join("src/foo.rs");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "").unwrap();
        let input = json!({"file_path": target.to_str().unwrap()});
        let result = build_write_guard_response(
            Some(&t),
            "Write",
            WriteGuardContext {
                project_dir: Some(project.path()),
                tool_input: Some(&input),
            },
        );
        let body = result.expect("deny");
        assert!(body
            .hook_specific_output
            .permission_decision_reason
            .contains("inside the shared worktree"));
    }

    #[test]
    fn write_inside_oxplow_state_dir_denied() {
        let t = read_only_thread();
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(project.path().join(".oxplow/runtime")).unwrap();
        let target = project.path().join(".oxplow/runtime/local.sqlite");
        std::fs::write(&target, "").unwrap();
        let input = json!({"file_path": target.to_str().unwrap()});
        let result = build_write_guard_response(
            Some(&t),
            "Write",
            WriteGuardContext {
                project_dir: Some(project.path()),
                tool_input: Some(&input),
            },
        );
        assert!(result.is_some(), ".oxplow runtime dir should be denied");
    }
}
