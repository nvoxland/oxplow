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

use std::path::{Path, PathBuf};

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
    /// The command bus: the command does not admit agents.
    Command,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Deny { layer: DenyLayer, reason: String },
}

/// The facts a decision needs, gathered by the caller.
pub struct PolicyFacts<'a> {
    pub thread: &'a Thread,
    /// The thread's own stream worktree: where its write guard and filing
    /// apply, and what relative paths resolve against.
    pub worktree_root: &'a Path,
    /// Every other stream's worktree (the primary checkout included). No
    /// thread edits them (workspace isolation).
    pub other_roots: &'a [PathBuf],
    /// The primary project, whose `.oxplow/wiki` every stream shares.
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

/// `path` made absolute (against `base`), with `.`/`..` resolved and the
/// deepest existing ancestor canonicalized (symlinks, `/var` →
/// `/private/var`), so a path can't dodge a prefix check by spelling.
pub fn normalize_path(path: &Path, base: &Path) -> PathBuf {
    use std::path::Component;
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let mut lexical = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::ParentDir => {
                lexical.pop();
            }
            Component::CurDir => {}
            other => lexical.push(other.as_os_str()),
        }
    }
    // Canonicalize the longest existing prefix, then re-append the rest.
    let mut existing = lexical.clone();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(canon) = existing.canonicalize() {
            let mut out = canon;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (
            existing.file_name().map(|n| n.to_os_string()),
            existing.parent(),
        ) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                existing = parent.to_path_buf();
            }
            _ => return lexical,
        }
    }
}

/// The other stream's worktree `path` falls in, if any. The shared wiki
/// (`<project>/.oxplow/wiki`) belongs to every stream.
fn foreign_root<'a>(path: &str, facts: &'a PolicyFacts<'_>) -> Option<&'a Path> {
    let p = Path::new(path);
    if !p.is_absolute() || p.starts_with(facts.worktree_root) {
        return None;
    }
    if p.starts_with(facts.project_dir.join(".oxplow").join("wiki")) {
        return None;
    }
    facts
        .other_roots
        .iter()
        .map(PathBuf::as_path)
        .find(|root| p.starts_with(root))
}

/// Decide whether `intent` may run. The write guard is checked before
/// filing; with several paths, the first path either rule refuses wins.
pub fn decide_tool(intent: &ToolIntent<'_>, facts: &PolicyFacts<'_>) -> PolicyDecision {
    if intent.kind != IntentKind::WorktreeWrite {
        return PolicyDecision::Allow;
    }
    // Compare real locations, not spellings.
    let own = normalize_path(facts.worktree_root, facts.worktree_root);
    let others: Vec<PathBuf> = facts
        .other_roots
        .iter()
        .map(|r| normalize_path(r, r))
        .collect();
    let project = normalize_path(facts.project_dir, facts.project_dir);
    let resolved: Vec<String> = intent
        .paths
        .iter()
        .map(|p| {
            normalize_path(Path::new(p), &own)
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let facts = &PolicyFacts {
        thread: facts.thread,
        worktree_root: &own,
        other_roots: &others,
        project_dir: &project,
        has_in_progress_claim: facts.has_in_progress_claim,
        git_operation_in_progress: facts.git_operation_in_progress,
    };
    let intent = &ToolIntent {
        label: intent.label,
        kind: intent.kind,
        paths: &resolved,
    };
    // Another stream's tree is off limits to every thread, writer or not.
    for p in intent.paths {
        if let Some(root) = foreign_root(p, facts) {
            return PolicyDecision::Deny {
                layer: DenyLayer::WriteGuard,
                reason: format!(
                    "path `{p}` is in another stream's worktree (`{}`). A thread may edit only \
                     its own stream's worktree; do this from that stream's thread, or record it \
                     as a note on the current task.",
                    root.display()
                ),
            };
        }
    }
    let targets: Vec<Option<&str>> = if intent.paths.is_empty() {
        vec![None]
    } else {
        intent
            .paths
            .iter()
            .map(String::as_str)
            .filter(|p| !path_outside_worktree(p, facts.worktree_root))
            .map(Some)
            .collect()
    };
    for t in &targets {
        if let Some(reason) = read_only_reason(facts.thread, *t, Some(facts.worktree_root)) {
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
                worktree_root: Path::new("/proj"),
                other_roots: &[std::path::PathBuf::from("/proj-wt")],
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
            worktree_root: Path::new("/proj"),
            other_roots: &[],
            project_dir: Path::new("/proj"),
            has_in_progress_claim: false,
            git_operation_in_progress: true,
        };
        assert_eq!(decide_tool(&intent, &facts), PolicyDecision::Allow);
    }

    /// A thread in a worktree stream: its worktree is a sibling of the
    /// primary checkout (`/proj-wt` next to `/proj`).
    fn decide_in_worktree(status: ThreadStatus, claim: bool, paths: &[&str]) -> PolicyDecision {
        let t = thread(status);
        let paths: Vec<String> = paths.iter().map(|p| p.to_string()).collect();
        decide_tool(
            &ToolIntent {
                label: "Edit",
                kind: IntentKind::WorktreeWrite,
                paths: &paths,
            },
            &PolicyFacts {
                thread: &t,
                worktree_root: Path::new("/proj-wt"),
                other_roots: &[std::path::PathBuf::from("/proj")],
                project_dir: Path::new("/proj"),
                has_in_progress_claim: claim,
                git_operation_in_progress: false,
            },
        )
    }

    #[test]
    fn a_worktree_streams_own_tree_is_guarded() {
        // Read-only: denied inside its own worktree (relative or absolute).
        for p in ["/proj-wt/src/a.rs", "src/a.rs"] {
            assert_eq!(
                layer(&decide_in_worktree(ThreadStatus::Queued, true, &[p])),
                Some(DenyLayer::WriteGuard),
                "{p}"
            );
        }
        // The writer still needs a claim there.
        assert_eq!(
            layer(&decide_in_worktree(
                ThreadStatus::Active,
                false,
                &["/proj-wt/src/a.rs"]
            )),
            Some(DenyLayer::Filing)
        );
        assert_eq!(
            decide_in_worktree(ThreadStatus::Active, true, &["/proj-wt/src/a.rs"]),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn no_thread_edits_another_streams_worktree() {
        let d = decide_in_worktree(ThreadStatus::Active, true, &["/proj/src/a.rs"]);
        let PolicyDecision::Deny { layer: lyr, reason } = d else {
            panic!("should deny")
        };
        assert_eq!(lyr, DenyLayer::WriteGuard);
        assert!(reason.contains("another stream"), "{reason}");
        // The primary's thread can't edit a worktree stream's tree either.
        assert_eq!(
            layer(&decide(
                ThreadStatus::Active,
                true,
                "Edit",
                IntentKind::WorktreeWrite,
                &["/proj-wt/src/a.rs"]
            )),
            Some(DenyLayer::WriteGuard)
        );
        // The shared wiki lives in the primary project and stays writable.
        assert_eq!(
            decide_in_worktree(ThreadStatus::Queued, false, &["/proj/.oxplow/wiki/x.md"]),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn dot_dot_and_symlinks_cant_smuggle_a_path_out_of_the_guard() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap().join("proj");
        std::fs::create_dir_all(root.join("src")).unwrap();
        let elsewhere = dir.path().canonicalize().unwrap().join("tmp");
        std::fs::create_dir_all(&elsewhere).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&root, elsewhere.join("lnk")).unwrap();
        let t = thread(ThreadStatus::Queued);
        let mut paths = vec![format!("{}/../proj/src/a.rs", elsewhere.display())];
        #[cfg(unix)]
        paths.push(format!("{}/lnk/src/new.rs", elsewhere.display()));
        for p in paths {
            let one = vec![p.clone()];
            let d = decide_tool(
                &ToolIntent {
                    label: "Write",
                    kind: IntentKind::WorktreeWrite,
                    paths: &one,
                },
                &PolicyFacts {
                    thread: &t,
                    worktree_root: &root,
                    other_roots: &[],
                    project_dir: &root,
                    has_in_progress_claim: true,
                    git_operation_in_progress: false,
                },
            );
            assert_eq!(layer(&d), Some(DenyLayer::WriteGuard), "{p}");
        }
    }
}
