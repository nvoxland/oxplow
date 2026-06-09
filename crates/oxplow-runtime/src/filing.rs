//! Filing-enforcement guard.
//!
//! Direct port of `src/electron/filing-enforcement.ts`. Fires before
//! Edit/Write/MultiEdit/NotebookEdit when the writer thread has no
//! `in_progress` task to claim the change.
//!
//! Design notes (from the TS source, preserved verbatim because they
//! still apply):
//! - A `ready`-status filing call alone does NOT satisfy the guard.
//!   Only `in_progress` is a commitment to ship now.
//! - Bash is intentionally excluded.
//! - Edits during a git operation (merge / rebase / cherry-pick /
//!   revert) are exempt.
//! - Read-only threads are out of scope here — `WriteGuard` runs
//!   first.

use serde::Serialize;
use specta::Type;

use oxplow_domain::Thread;

pub static ALWAYS_WRITE_INTENT_TOOL_NAMES: &[&str] =
    &["Write", "Edit", "MultiEdit", "NotebookEdit"];

#[derive(Debug, Clone, PartialEq, Serialize, Type)]
pub struct FilingEnforcementDeny {
    #[serde(rename = "hookSpecificOutput")]
    pub hook_specific_output: super::write_guard::HookSpecificOutput,
}

#[derive(Debug, Clone, Default)]
pub struct FilingEnforcementContext<'a> {
    pub thread: Option<&'a Thread>,
    pub tool_name: &'a str,
    pub has_in_progress_task: bool,
    /// Absolute path being written, when the tool input carries one.
    pub file_path: Option<&'a str>,
    pub git_operation_in_progress: bool,
}

/// Returns true when the path is under `~/.claude/plans/<slug>.md`.
/// The harness owns that directory; the carve-out exists so plan
/// mode's plan file isn't blocked by filing enforcement.
pub fn is_plan_mode_plan_file(file_path: Option<&str>) -> bool {
    let Some(path) = file_path else { return false };
    let Some(home) = std::env::var_os("HOME") else {
        return false;
    };
    let prefix = std::path::Path::new(&home).join(".claude").join("plans");
    let prefix_str = prefix.to_string_lossy().into_owned() + "/";
    path.starts_with(prefix_str.as_str()) && path.ends_with(".md")
}

pub fn build_filing_enforcement_pre_tool_deny(
    ctx: FilingEnforcementContext<'_>,
) -> Option<FilingEnforcementDeny> {
    let thread = ctx.thread?;
    // Filing enforcement only applies to the writer (active) thread.
    // Queued threads can't write at all (write-guard runs first);
    // closed threads can't either.
    if !thread.status.is_writer() {
        return None;
    }
    if !ALWAYS_WRITE_INTENT_TOOL_NAMES.contains(&ctx.tool_name) {
        return None;
    }
    if ctx.has_in_progress_task {
        return None;
    }
    if is_plan_mode_plan_file(ctx.file_path) {
        return None;
    }
    if ctx.git_operation_in_progress {
        return None;
    }
    Some(FilingEnforcementDeny {
        hook_specific_output: super::write_guard::HookSpecificOutput {
            hook_event_name: "PreToolUse",
            permission_decision: "deny",
            permission_decision_reason: build_filing_enforcement_pre_tool_reason(ctx.tool_name),
        },
    })
}

pub fn build_filing_enforcement_pre_tool_reason(tool_name: &str) -> String {
    [
        format!("BLOCKED: {tool_name} requires a tracked task on this thread before edits can land."),
        String::new(),
        "No `in_progress` task exists on this thread. `ready`-status rows don't count — `ready` is backlog, `in_progress` is the actual claim. The Work panel needs to honestly reflect what's shipping while it ships, not after.".into(),
        String::new(),
        "Pick one before re-issuing the edit:".into(),
        format!("  • New concern → `mcp__oxplow__create_task` with status=in_progress, then re-run {tool_name}. Close to done via `complete_task` when settled."),
        format!("  • Fix/redo of a recently-closed done item → `mcp__oxplow__update_task` → status=in_progress on that item, then re-run {tool_name}. Close back to done when settled."),
        "  • Already dispatched against a ready row → `mcp__oxplow__update_task` → status=in_progress on that row first.".into(),
        String::new(),
        "Do not file a placeholder \"untracked work\" item — describe the real change you're about to make.".into(),
    ]
    .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::{StreamId, ThreadId, ThreadStatus, Timestamp};

    fn open_thread() -> Thread {
        Thread {
            id: ThreadId::new(1),
            stream_id: StreamId::new(1),
            title: "explore".into(),
            status: ThreadStatus::Active,
            sort_index: 0,
            pane_target: "working".into(),
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

    #[test]
    fn no_thread_means_no_deny() {
        let result = build_filing_enforcement_pre_tool_deny(FilingEnforcementContext {
            tool_name: "Write",
            ..Default::default()
        });
        assert!(result.is_none());
    }

    #[test]
    fn read_tool_never_denied() {
        let t = open_thread();
        let result = build_filing_enforcement_pre_tool_deny(FilingEnforcementContext {
            thread: Some(&t),
            tool_name: "Read",
            ..Default::default()
        });
        assert!(result.is_none());
    }

    #[test]
    fn in_progress_item_satisfies() {
        let t = open_thread();
        let result = build_filing_enforcement_pre_tool_deny(FilingEnforcementContext {
            thread: Some(&t),
            tool_name: "Write",
            has_in_progress_task: true,
            ..Default::default()
        });
        assert!(result.is_none());
    }

    #[test]
    fn no_in_progress_item_denies_write() {
        let t = open_thread();
        let result = build_filing_enforcement_pre_tool_deny(FilingEnforcementContext {
            thread: Some(&t),
            tool_name: "Write",
            has_in_progress_task: false,
            ..Default::default()
        });
        let body = result.expect("deny");
        assert!(body
            .hook_specific_output
            .permission_decision_reason
            .contains("requires a tracked task"));
    }

    #[test]
    fn git_operation_in_progress_exempts() {
        let t = open_thread();
        let result = build_filing_enforcement_pre_tool_deny(FilingEnforcementContext {
            thread: Some(&t),
            tool_name: "Write",
            has_in_progress_task: false,
            git_operation_in_progress: true,
            ..Default::default()
        });
        assert!(result.is_none(), "merge edits should be exempt");
    }

    #[test]
    fn plan_mode_plan_file_exempts() {
        let t = open_thread();
        // HOME-based path, so we synthesize a path under it.
        let home = std::env::var("HOME").expect("HOME set on test runner");
        let path = format!("{home}/.claude/plans/foo.md");
        let result = build_filing_enforcement_pre_tool_deny(FilingEnforcementContext {
            thread: Some(&t),
            tool_name: "Write",
            has_in_progress_task: false,
            file_path: Some(&path),
            ..Default::default()
        });
        assert!(result.is_none(), "plan-mode plan file should be exempt");
    }

    #[test]
    fn other_dot_claude_paths_not_exempt() {
        let t = open_thread();
        let home = std::env::var("HOME").expect("HOME set");
        let path = format!("{home}/.claude/CLAUDE.md");
        let result = build_filing_enforcement_pre_tool_deny(FilingEnforcementContext {
            thread: Some(&t),
            tool_name: "Write",
            has_in_progress_task: false,
            file_path: Some(&path),
            ..Default::default()
        });
        assert!(
            result.is_some(),
            "non-plans paths under .claude should still be subject to filing enforcement"
        );
    }
}
