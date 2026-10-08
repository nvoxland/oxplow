//! The policy every agent transport enforces: the write guard (isolation
//! only), with the I/O it needs (the thread, the stream worktrees).
//! Decisions are transport-neutral: the hook route renders them as
//! Claude's JSON, and the ACP client as permission answers. The pure rules
//! live in `oxplow_runtime::policy`. Neither an edit nor a turn's end ever
//! waits on tracked work (`.context/work-tracking.md`). See
//! `.context/agent-model.md`.

use oxplow_domain::agent::tool::{ToolKind, ToolUse};
use oxplow_domain::stores::ThreadStore;
use oxplow_domain::ThreadId;
use oxplow_runtime::policy::{decide_tool, IntentKind, PolicyDecision, PolicyFacts, ToolIntent};

use crate::Services;

/// The shared agent policy (`Services.agent_policy`).
#[derive(Default)]
pub struct AgentPolicy;

/// A tool call as the policy sees it: an edit names the worktree files it
/// writes; every other kind is one neither rule refuses.
pub fn intent_of(tool: &ToolUse) -> ToolIntent<'_> {
    ToolIntent {
        label: &tool.name,
        kind: if tool.kind == ToolKind::Edit {
            IntentKind::WorktreeWrite
        } else {
            IntentKind::Other
        },
        paths: &tool.paths,
    }
}

/// Whether the policy could refuse `tool`: only an edit. A caller skips
/// the policy's lookups for anything else.
pub fn may_refuse(tool: &ToolUse) -> bool {
    tool.kind == ToolKind::Edit
}

impl AgentPolicy {
    /// May an agent run `spec`? The bus has already admitted the invoker;
    /// this is the agent-specific layer on top: a `Write` command needs a
    /// thread that may write (`may_write`, from the bus's write gate —
    /// a queued or closed thread can read, not change). `None` means the
    /// gate had nothing to say (a read, or no thread/gate).
    pub fn check_command(
        &self,
        thread_id: Option<&ThreadId>,
        spec: &oxplow_domain::CommandSpec,
        may_write: Option<bool>,
    ) -> PolicyDecision {
        if !spec.invokers.agent {
            return PolicyDecision::Deny {
                layer: oxplow_runtime::policy::DenyLayer::Command,
                reason: format!("`{}` is not open to agents", spec.id),
            };
        }
        if may_write == Some(false) {
            return PolicyDecision::Deny {
                layer: oxplow_runtime::policy::DenyLayer::Command,
                reason: format!(
                    "`{}` changes state, and thread {} may not write (only the stream's writer thread can)",
                    spec.id,
                    thread_id.map(|t| t.to_string()).unwrap_or_default()
                ),
            };
        }
        PolicyDecision::Allow
    }

    /// May `thread_id` run `intent` now? Allows when the thread is
    /// unknown (nothing to enforce against).
    pub async fn check_tool(
        &self,
        svc: &Services,
        thread_id: &ThreadId,
        intent: &ToolIntent<'_>,
    ) -> PolicyDecision {
        if intent.kind != IntentKind::WorktreeWrite {
            return PolicyDecision::Allow;
        }
        let Some(thread) = svc.thread_store.get(thread_id).await.ok().flatten() else {
            return PolicyDecision::Allow;
        };
        let project_dir = svc.layout.project_dir.as_path();
        // The thread's own tree is its stream's worktree (a sibling dir for
        // a worktree stream); every other stream's tree is off limits.
        let streams = oxplow_domain::stores::StreamStore::list(svc.stream_store.as_ref())
            .await
            .unwrap_or_default();
        let worktree_root = svc.thread_worktree(thread_id).await;
        let other_roots: Vec<std::path::PathBuf> = streams
            .iter()
            .filter(|s| s.id != thread.stream_id && !s.worktree_path.is_empty())
            .map(|s| std::path::PathBuf::from(&s.worktree_path))
            .chain(std::iter::once(project_dir.to_path_buf()))
            .filter(|p| p != &worktree_root)
            .collect();
        decide_tool(
            intent,
            &PolicyFacts {
                thread: &thread,
                worktree_root: &worktree_root,
                other_roots: &other_roots,
                project_dir,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(kind: ToolKind, paths: &[&str]) -> ToolUse {
        ToolUse {
            name: "apply_patch".into(),
            kind,
            paths: paths.iter().map(|p| p.to_string()).collect(),
            ..ToolUse::default()
        }
    }

    /// An edit is a worktree write of every file it names, whatever its
    /// harness calls it; nothing else can be refused.
    #[test]
    fn only_an_edit_is_a_worktree_write_of_every_file_it_names() {
        let edit = call(ToolKind::Edit, &["src/a.rs", "src/b.rs"]);
        assert!(may_refuse(&edit));
        let i = intent_of(&edit);
        assert_eq!(
            (i.label, i.kind, i.paths.to_vec()),
            (
                "apply_patch",
                IntentKind::WorktreeWrite,
                vec!["src/a.rs".to_string(), "src/b.rs".to_string()]
            )
        );
        for kind in [
            ToolKind::Read,
            ToolKind::Shell,
            ToolKind::Subagent,
            ToolKind::Search,
            ToolKind::Mcp,
            ToolKind::Other,
        ] {
            assert!(!may_refuse(&call(kind, &["x"])), "{kind:?}");
            assert_eq!(intent_of(&call(kind, &["x"])).kind, IntentKind::Other);
        }
    }
}
