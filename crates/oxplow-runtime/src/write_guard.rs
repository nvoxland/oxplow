//! Write guard for read-only threads.
//!
//! Direct port of `src/electron/write-guard.ts`. Returns a
//! Claude-Code-shaped PreToolUse deny body when the calling thread is
//! not the stream's writer and the tool would mutate the shared
//! worktree.

use std::path::{Path, PathBuf};
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
    if thread.status.is_writer() {
        return None;
    }
    if tool_name.is_empty() {
        return None;
    }
    if tool_name.starts_with("mcp__") {
        return None;
    }
    if !WORKTREE_MUTATING_TOOLS.contains(&tool_name) {
        return None;
    }

    if let (Some(project_dir), Some(tool_input)) = (context.project_dir, context.tool_input) {
        if let Some(abs) = extract_abs_target_path(tool_input, project_dir) {
            let oxplow_dir = project_dir.join(".oxplow");
            let notes_dir = oxplow_dir.join("wiki");
            let inside_project = is_inside(&abs, project_dir);
            let inside_oxplow = is_inside(&abs, &oxplow_dir);
            let inside_notes = is_inside(&abs, &notes_dir);
            if inside_notes {
                return None;
            }
            if !inside_project && !inside_oxplow {
                return None;
            }
            return Some(WriteGuardDeny {
                hook_specific_output: HookSpecificOutput {
                    hook_event_name: "PreToolUse",
                    permission_decision: "deny",
                    permission_decision_reason: format!(
                        "path `{}` is inside the shared worktree and this thread is read-only — \
                         only the stream's writer thread may mutate the worktree. \
                         Record the change as a note on the current task via mcp__oxplow tools (or stop this turn). \
                         Promote this thread to writer from the thread rail if you need to edit.",
                        abs.display()
                    ),
                },
            });
        }
    }

    Some(WriteGuardDeny {
        hook_specific_output: HookSpecificOutput {
            hook_event_name: "PreToolUse",
            permission_decision: "deny",
            permission_decision_reason:
                "This thread is read-only — only the stream's writer thread may mutate the worktree. \
                 Record the change as a note on the current task via mcp__oxplow tools (or stop this turn). \
                 Promote this thread to writer from the thread rail if you need to edit."
                    .into(),
        },
    })
}

fn extract_abs_target_path(tool_input: &Value, project_dir: &Path) -> Option<PathBuf> {
    let raw = tool_input
        .get("file_path")
        .and_then(|v| v.as_str())
        .or_else(|| tool_input.get("notebook_path").and_then(|v| v.as_str()))
        .or_else(|| tool_input.get("path").and_then(|v| v.as_str()))?;
    let path = Path::new(raw);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        project_dir.join(path)
    })
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
            "mcp__oxplow__create_task",
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
    fn write_to_notes_dir_allowed() {
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
        assert!(result.is_none(), "notes dir should be allowed");
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
