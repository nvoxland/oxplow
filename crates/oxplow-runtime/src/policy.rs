//! The tool policy every agent transport asks (tsk333): may this thread
//! run this tool now? It combines the write guard (read-only threads
//! can't touch the worktree) and filing enforcement (the writer needs an
//! `in_progress` task). The hook route asks it with Claude-shaped
//! payloads; the ACP client asks it with ACP tool calls. The answer
//! carries the reason text only; each transport renders it in its own
//! wire shape. See `.context/agent-model.md` → "Write guard".
//!
//! Pure: the facts (thread, whether the stream has a claim, git state)
//! are gathered by the caller (`oxplow_app::agent_policy`).

use std::path::Path;

use oxplow_domain::Thread;

use crate::filing::filing_reason;
use crate::write_guard::read_only_reason;

/// What a tool call would do, as far as the policy cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentKind {
    /// Changes files in the worktree (edit, write, delete, move).
    WorktreeWrite,
    /// Anything else (reads, searches, commands, MCP calls).
    Other,
}

/// A tool call as the policy sees it.
#[derive(Debug, Clone)]
pub struct ToolIntent<'a> {
    /// The agent's own name for the tool (`Edit`, `Delete`, …), used
    /// verbatim in reasons.
    pub label: &'a str,
    pub kind: IntentKind,
    /// Target paths, absolute or project-relative. Empty when the call
    /// names none.
    pub paths: &'a [String],
}

/// Which rule denied it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyLayer {
    WriteGuard,
    Filing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Deny { layer: DenyLayer, reason: String },
}

/// The facts a decision needs, gathered by the caller.
pub struct PolicyFacts<'a> {
    pub thread: &'a Thread,
    pub project_dir: &'a Path,
    /// Some thread in the stream has an `in_progress` task.
    pub has_in_progress_claim: bool,
    /// A merge / rebase / cherry-pick / revert is underway.
    pub git_operation_in_progress: bool,
}

/// An absolute path outside the project: not oxplow's concern (it can't
/// be a project file or an effort's change), so neither rule applies.
pub fn path_outside_worktree(path: &str, project_dir: &Path) -> bool {
    let p = Path::new(path);
    p.is_absolute() && !p.starts_with(project_dir)
}

/// Decide whether `intent` may run. The write guard is checked before
/// filing; with several paths, the first path either rule refuses wins.
pub fn decide_tool(intent: &ToolIntent<'_>, facts: &PolicyFacts<'_>) -> PolicyDecision {
    if intent.kind != IntentKind::WorktreeWrite {
        return PolicyDecision::Allow;
    }
    let targets: Vec<Option<&str>> = if intent.paths.is_empty() {
        vec![None]
    } else {
        intent
            .paths
            .iter()
            .map(String::as_str)
            .filter(|p| !path_outside_worktree(p, facts.project_dir))
            .map(Some)
            .collect()
    };
    for t in &targets {
        if let Some(reason) = read_only_reason(facts.thread, *t, Some(facts.project_dir)) {
            return PolicyDecision::Deny {
                layer: DenyLayer::WriteGuard,
                reason,
            };
        }
    }
    for t in &targets {
        if let Some(reason) = filing_reason(
            facts.thread,
            intent.label,
            facts.has_in_progress_claim,
            *t,
            facts.git_operation_in_progress,
        ) {
            return PolicyDecision::Deny {
                layer: DenyLayer::Filing,
                reason,
            };
        }
    }
    PolicyDecision::Allow
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::{StreamId, ThreadId, ThreadStatus, Timestamp};

    fn thread(status: ThreadStatus) -> Thread {
        Thread {
            id: ThreadId::new(2),
            stream_id: StreamId::new(1),
            title: "t".into(),
            status,
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

    fn decide(
        status: ThreadStatus,
        claim: bool,
        label: &str,
        kind: IntentKind,
        paths: &[&str],
    ) -> PolicyDecision {
        let t = thread(status);
        let paths: Vec<String> = paths.iter().map(|p| p.to_string()).collect();
        decide_tool(
            &ToolIntent {
                label,
                kind,
                paths: &paths,
            },
            &PolicyFacts {
                thread: &t,
                project_dir: Path::new("/proj"),
                has_in_progress_claim: claim,
                git_operation_in_progress: false,
            },
        )
    }

    fn layer(d: &PolicyDecision) -> Option<DenyLayer> {
        match d {
            PolicyDecision::Deny { layer, .. } => Some(*layer),
            PolicyDecision::Allow => None,
        }
    }

    #[test]
    fn a_read_only_thread_cannot_write_inside_the_worktree() {
        let d = decide(
            ThreadStatus::Queued,
            true,
            "Edit",
            IntentKind::WorktreeWrite,
            &["/proj/src/a.rs"],
        );
        assert_eq!(layer(&d), Some(DenyLayer::WriteGuard));
        let PolicyDecision::Deny { reason, .. } = d else {
            unreachable!()
        };
        assert!(
            reason.contains("/proj/src/a.rs") && reason.contains("read-only"),
            "{reason}"
        );
        // No path at all: the generic read-only reason.
        let d = decide(
            ThreadStatus::Queued,
            true,
            "Write",
            IntentKind::WorktreeWrite,
            &[],
        );
        assert_eq!(layer(&d), Some(DenyLayer::WriteGuard));
    }

    #[test]
    fn the_writer_needs_a_claim_and_the_reason_names_the_tool() {
        let d = decide(
            ThreadStatus::Active,
            false,
            "Delete",
            IntentKind::WorktreeWrite,
            &["src/a.rs"],
        );
        let PolicyDecision::Deny { layer, reason } = d else {
            panic!("should deny")
        };
        assert_eq!(layer, DenyLayer::Filing);
        assert!(
            reason.starts_with("BLOCKED: Delete requires a tracked task"),
            "{reason}"
        );
        assert_eq!(
            decide(
                ThreadStatus::Active,
                true,
                "Delete",
                IntentKind::WorktreeWrite,
                &["src/a.rs"]
            ),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn outside_paths_and_non_writes_are_allowed() {
        assert_eq!(
            decide(
                ThreadStatus::Queued,
                false,
                "Edit",
                IntentKind::WorktreeWrite,
                &["/tmp/x"]
            ),
            PolicyDecision::Allow
        );
        assert_eq!(
            decide(
                ThreadStatus::Queued,
                false,
                "Read",
                IntentKind::Other,
                &["/proj/a"]
            ),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn any_denied_path_denies_a_multi_path_call() {
        // A move from outside into the worktree still writes the worktree.
        let d = decide(
            ThreadStatus::Queued,
            true,
            "Move",
            IntentKind::WorktreeWrite,
            &["/tmp/x", "/proj/y"],
        );
        assert_eq!(layer(&d), Some(DenyLayer::WriteGuard));
    }

    #[test]
    fn a_git_operation_exempts_filing_but_not_the_write_guard() {
        let t = thread(ThreadStatus::Active);
        let paths = vec!["src/a.rs".to_string()];
        let intent = ToolIntent {
            label: "Edit",
            kind: IntentKind::WorktreeWrite,
            paths: &paths,
        };
        let facts = PolicyFacts {
            thread: &t,
            project_dir: Path::new("/proj"),
            has_in_progress_claim: false,
            git_operation_in_progress: true,
        };
        assert_eq!(decide_tool(&intent, &facts), PolicyDecision::Allow);
    }
}
