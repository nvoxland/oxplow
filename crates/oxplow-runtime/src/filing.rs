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
    pub has_open_effort: bool,
    /// Absolute path being written, when the tool input carries one.
    pub file_path: Option<&'a str>,
    pub git_operation_in_progress: bool,
    /// The project's active work-items provider (`oxplow` unless
    /// `activeProviders` names another); `None` reads as oxplow.
    pub active_work_items: Option<&'a str>,
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
    if !ALWAYS_WRITE_INTENT_TOOL_NAMES.contains(&ctx.tool_name) {
        return None;
    }
    filing_reason(
        thread,
        ctx.tool_name,
        ctx.has_open_effort,
        ctx.file_path,
        ctx.git_operation_in_progress,
        ctx.active_work_items.unwrap_or(OXPLOW),
    )
    .map(|reason| FilingEnforcementDeny {
        hook_specific_output: super::write_guard::HookSpecificOutput {
            hook_event_name: "PreToolUse",
            permission_decision: "deny",
            permission_decision_reason: reason,
        },
    })
}

/// Filing enforcement's reason, if the writer `thread` may not write
/// `file_path` with the tool the agent calls `label` yet. The core shared
/// by the Claude hook response and [`crate::policy::decide_tool`]. Only
/// the writer is in scope (queued/closed threads can't write at all:
/// the write guard runs first).
pub fn filing_reason(
    thread: &Thread,
    label: &str,
    has_open_effort: bool,
    file_path: Option<&str>,
    git_operation_in_progress: bool,
    active_work_items: &str,
) -> Option<String> {
    if !thread.status.is_writer()
        || has_open_effort
        || is_plan_mode_plan_file(file_path)
        || git_operation_in_progress
    {
        return None;
    }
    Some(build_filing_enforcement_pre_tool_reason(
        label,
        thread.id,
        active_work_items,
    ))
}

/// oxplow's own work-items provider.
const OXPLOW: &str = "oxplow";

/// The reason, naming the project's active work-items provider when it
/// isn't oxplow's own: new work belongs on it (P7.A2). Every fix is a
/// command (`mcp__oxplow__run_command`, P8.A10); `thread` is the agent's
/// own, which a new task is filed on.
pub fn build_filing_enforcement_pre_tool_reason(
    tool_name: &str,
    thread: oxplow_domain::ThreadId,
    active_work_items: &str,
) -> String {
    let mut lines = vec![
        format!("BLOCKED: {tool_name} requires open, tracked work in this stream before edits can land."),
        String::new(),
        "No effort is open in this stream. An effort opens when a task goes `in_progress` — `ready`-status rows don't count: `ready` is backlog, `in_progress` is the actual claim. The Work panel needs to honestly reflect what's shipping while it ships, not after.".into(),
        String::new(),
        "Pick one before re-issuing the edit — each is `mcp__oxplow__run_command`:".into(),
        format!("  • New concern → `work_item.create` with `{{\"title\": …, \"body\": …, \"state\": \"in_progress\", \"native\": {{\"thread\": \"{thread}\"}}}}`, then re-run {tool_name}. When it ships: `command.sequence` of `work_item.transition` (`\"to\": \"done\"`) and `effort.report` (`summary`, `touched_files`)."),
        format!("  • Fix/redo of a recently-closed done item → `work_item.transition` with `{{\"ref\": \"work_item:oxplow:tsk…\", \"to\": \"in_progress\"}}`, then re-run {tool_name}. Close it back to done when settled."),
        "  • Already dispatched against a ready row → `work_item.transition` it to `in_progress` first.".into(),
        format!("  • Work tracked outside oxplow (a Linear/GitHub issue) → `effort.open` with `{{\"work_item\": \"work_item:<provider>:<id>\"}}`, then re-run {tool_name}; `effort.close` when done."),
        String::new(),
        "Do not file a placeholder \"untracked work\" item — describe the real change you're about to make.".into(),
    ];
    if active_work_items != OXPLOW {
        lines.splice(
            2..2,
            [
                format!("This project's work items live on `{active_work_items}` (its active provider). File a new concern there — `mcp__oxplow__run_command` `work_item.create` with `{{\"title\": …}}` files on `{active_work_items}` — then `effort.open` on the ref it returns, and re-run {tool_name}; `effort.close` when done. The `work_item:oxplow` lines below file oxplow tasks."),
                String::new(),
            ],
        );
    }
    lines.join("\n")
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
            has_open_effort: true,
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
            has_open_effort: false,
            ..Default::default()
        });
        let body = result.expect("deny");
        assert!(body
            .hook_specific_output
            .permission_decision_reason
            .contains("requires open, tracked work"));
    }

    #[test]
    fn git_operation_in_progress_exempts() {
        let t = open_thread();
        let result = build_filing_enforcement_pre_tool_deny(FilingEnforcementContext {
            thread: Some(&t),
            tool_name: "Write",
            has_open_effort: false,
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
            has_open_effort: false,
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
            has_open_effort: false,
            file_path: Some(&path),
            ..Default::default()
        });
        assert!(
            result.is_some(),
            "non-plans paths under .claude should still be subject to filing enforcement"
        );
    }

    /// P7.A2: with another active work-items provider, the directive
    /// says where new work belongs and how to file it there.
    #[test]
    fn the_directive_names_another_active_provider() {
        let thread = oxplow_domain::ThreadId::new(4);
        let oxplow = build_filing_enforcement_pre_tool_reason("Edit", thread, "oxplow");
        assert!(!oxplow.contains("active provider"), "{oxplow}");
        // P8.A10: the fixes are commands; a new task goes on the agent's
        // own thread.
        assert!(
            oxplow.contains("`work_item.create`") && oxplow.contains("\"thread\": \"thr4\""),
            "{oxplow}"
        );
        assert!(!oxplow.contains("create_task"), "{oxplow}");
        let linear = build_filing_enforcement_pre_tool_reason("Edit", thread, "linear");
        assert!(
            linear.contains("work items live on `linear` (its active provider)")
                && linear.contains("`work_item.create`")
                && linear.contains("effort.open"),
            "{linear}"
        );
    }
}
