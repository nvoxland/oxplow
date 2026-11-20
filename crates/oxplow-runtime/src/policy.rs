//! The tool policy every agent transport asks: may this thread run this
//! tool now? It is isolation only — the write guard (no thread touches
//! another stream's worktree; a store's row files in a worktree are written by command). It never
//! asks for tracked work (`.context/work-tracking.md`). The hook route asks
//! it with Claude-shaped payloads; the Codex observer asks it with Codex
//! item kinds mapped to tool calls. The answer carries the reason text only; each transport renders
//! it in its own wire shape. See `.context/agent-model.md` → "Isolation".
//!
//! Pure: the facts (the worktrees) are gathered by the caller
//! (`oxplow_app::agent_policy`).

use std::path::{Path, PathBuf};

use crate::write_guard::{foreign_worktree_reason, record_file_reason, RecordFolder};

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
    /// The thread's own stream worktree: what relative paths resolve
    /// against, and the one tree it may write.
    pub worktree_root: &'a Path,
    /// Every other stream's worktree (the primary checkout included). No
    /// thread edits them (workspace isolation).
    pub other_roots: &'a [PathBuf],
    /// The primary project, whose stores' row files (`record_folders`) every
    /// stream shares.
    pub project_dir: &'a Path,
    /// The files stores keep their rows in (the fills' `files:`): written by
    /// the store's extension's command, never by a tool.
    pub record_folders: &'a [RecordFolder],
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

/// Decide whether `intent` may run; with several paths, the first path
/// refused wins.
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
    // A store's row file in a worktree is written by its command: only the
    // primary checkout's folder is followed.
    for p in &resolved {
        if let Some(reason) =
            record_file_reason(Some(p), Some(&own), Some(&project), facts.record_folders)
        {
            return PolicyDecision::Deny {
                layer: DenyLayer::WriteGuard,
                reason,
            };
        }
    }
    // Another stream's tree is off limits to every thread.
    for p in &resolved {
        if let Some(reason) = foreign_worktree_reason(p, &own, &others) {
            return PolicyDecision::Deny {
                layer: DenyLayer::WriteGuard,
                reason,
            };
        }
    }
    PolicyDecision::Allow
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

    /// A call from a thread of the stream at `own`, the other stream's
    /// tree at `other`, the project at `/proj`.
    fn decide_at(own: &str, other: &str, kind: IntentKind, paths: &[&str]) -> PolicyDecision {
        let paths: Vec<String> = paths.iter().map(|p| p.to_string()).collect();
        decide_tool(
            &ToolIntent {
                label: "Edit",
                kind,
                paths: &paths,
            },
            &PolicyFacts {
                worktree_root: Path::new(own),
                other_roots: &[std::path::PathBuf::from(other)],
                project_dir: Path::new("/proj"),
                record_folders: &oxwiki(),
            },
        )
    }

    /// From the primary's thread (`/proj`, a worktree stream at `/proj-wt`).
    fn decide(kind: IntentKind, paths: &[&str]) -> PolicyDecision {
        decide_at("/proj", "/proj-wt", kind, paths)
    }

    /// From a worktree stream's thread (`/proj-wt`, the primary at `/proj`).
    fn decide_in_worktree(paths: &[&str]) -> PolicyDecision {
        decide_at("/proj-wt", "/proj", IntentKind::WorktreeWrite, paths)
    }

    fn layer(d: &PolicyDecision) -> Option<DenyLayer> {
        match d {
            PolicyDecision::Deny { layer, .. } => Some(*layer),
            PolicyDecision::Allow => None,
        }
    }

    /// Any thread edits its own stream's worktree with nothing tracked —
    /// relative or absolute, or naming no path at all.
    #[test]
    fn a_thread_edits_its_own_worktree_without_tracked_work() {
        for paths in [&["src/a.rs"][..], &["/proj/src/a.rs"], &[]] {
            assert_eq!(
                decide(IntentKind::WorktreeWrite, paths),
                PolicyDecision::Allow,
                "{paths:?}"
            );
        }
        for p in ["/proj-wt/src/a.rs", "src/a.rs"] {
            assert_eq!(decide_in_worktree(&[p]), PolicyDecision::Allow, "{p}");
        }
    }

    #[test]
    fn outside_paths_and_non_writes_are_allowed() {
        assert_eq!(
            decide(IntentKind::WorktreeWrite, &["/tmp/x"]),
            PolicyDecision::Allow
        );
        assert_eq!(
            decide(IntentKind::Other, &["/proj-wt/a"]),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn any_denied_path_denies_a_multi_path_call() {
        // A move from its own tree into another stream's still writes there.
        let d = decide(IntentKind::WorktreeWrite, &["/proj/x", "/proj-wt/y"]);
        assert_eq!(layer(&d), Some(DenyLayer::WriteGuard));
    }

    #[test]
    fn no_thread_edits_another_streams_worktree() {
        let d = decide_in_worktree(&["/proj/src/a.rs"]);
        let PolicyDecision::Deny { layer: lyr, reason } = d else {
            panic!("should deny")
        };
        assert_eq!(lyr, DenyLayer::WriteGuard);
        assert!(reason.contains("another stream"), "{reason}");
        // The primary's thread can't edit a worktree stream's tree either.
        assert_eq!(
            layer(&decide(IntentKind::WorktreeWrite, &["/proj-wt/src/a.rs"])),
            Some(DenyLayer::WriteGuard)
        );
    }

    /// A store's row files: in the primary checkout a direct write is
    /// allowed (the store's collector converges it); in a worktree, the
    /// worktree's own copy of the folder is refused (only the primary is
    /// watched, so the page would be silently lost) naming the store's
    /// command, which writes to the primary from anywhere. Other files of
    /// the worktree are its thread's to write.
    #[test]
    fn a_store_file_is_written_directly_in_the_primary_and_by_command_in_a_worktree() {
        // The primary's thread writes the store's file itself.
        assert_eq!(
            decide(IntentKind::WorktreeWrite, &["/proj/.oxplow/oxwiki/x.md"]),
            PolicyDecision::Allow
        );
        // A worktree thread's own copy of the folder is refused, naming the
        // extension whose command writes it.
        let d = decide_in_worktree(&["/proj-wt/.oxplow/oxwiki/x.md"]);
        assert!(
            matches!(&d, PolicyDecision::Deny { layer: DenyLayer::WriteGuard, reason }
                if reason.contains("`oxwiki.*` commands")
                    && reason.contains("primary checkout")),
            "{d:?}"
        );
        // Its other files, and the folder's files that aren't rows, are its own.
        assert_eq!(
            decide_in_worktree(&["/proj-wt/src/a.rs", "/proj-wt/.oxplow/oxwiki/notes.txt"]),
            PolicyDecision::Allow
        );
        // `..` doesn't dodge the refusal.
        let d = decide_in_worktree(&["/proj-wt/src/../.oxplow/oxwiki/x.md"]);
        assert_eq!(layer(&d), Some(DenyLayer::WriteGuard), "{d:?}");
        // The primary's folder, from a worktree thread, is another stream's
        // tree: refused as before.
        let d = decide_in_worktree(&["/proj/.oxplow/oxwiki/x.md"]);
        assert!(
            matches!(&d, PolicyDecision::Deny { reason, .. } if reason.contains("another stream")),
            "{d:?}"
        );
    }

    /// A symlink in a worktree that leads into its store folder doesn't dodge
    /// the refusal either.
    #[test]
    fn a_symlink_into_a_worktrees_store_folder_is_refused_too() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let primary = base.join("proj");
        let worktree = base.join("proj-wt");
        std::fs::create_dir_all(worktree.join(".oxplow/oxwiki")).unwrap();
        std::fs::create_dir_all(&primary).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(worktree.join(".oxplow/oxwiki"), worktree.join("lnk"))
                .unwrap();
            let paths = vec![format!("{}/lnk/x.md", worktree.display())];
            let folders = oxwiki();
            let d = decide_tool(
                &ToolIntent {
                    label: "Write",
                    kind: IntentKind::WorktreeWrite,
                    paths: &paths,
                },
                &PolicyFacts {
                    worktree_root: &worktree,
                    other_roots: std::slice::from_ref(&primary),
                    project_dir: &primary,
                    record_folders: &folders,
                },
            );
            assert_eq!(layer(&d), Some(DenyLayer::WriteGuard), "{d:?}");
        }
    }

    /// A harness's worktree sits inside a checkout
    /// (`<repo>/.claude/worktrees/<name>`). Not adopted, it is part of the
    /// tree around it: its stream's agent edits it, another stream's can't.
    /// Adopted as a stream of its own, it belongs to that stream alone,
    /// though the primary's root is a prefix of its path.
    #[test]
    fn a_worktree_inside_a_checkout_belongs_to_the_deepest_stream() {
        let native = "/proj/.claude/worktrees/x/src/a.rs";
        // Not adopted: the primary's agent edits it; a worktree stream's can't.
        assert_eq!(
            decide_at("/proj", "/proj-wt", IntentKind::WorktreeWrite, &[native]),
            PolicyDecision::Allow
        );
        assert_eq!(
            layer(&decide_in_worktree(&[native])),
            Some(DenyLayer::WriteGuard)
        );
        // Adopted as the stream at `/proj/.claude/worktrees/x`: its own
        // agent edits it, the primary's can't.
        let adopted = [std::path::PathBuf::from("/proj/.claude/worktrees/x")];
        fn facts<'a>(
            own: &'a str,
            others: &'a [std::path::PathBuf],
            record_folders: &'a [RecordFolder],
        ) -> PolicyFacts<'a> {
            PolicyFacts {
                worktree_root: Path::new(own),
                other_roots: others,
                project_dir: Path::new("/proj"),
                record_folders,
            }
        }
        let folders = oxwiki();
        let paths = vec![native.to_string()];
        let intent = ToolIntent {
            label: "Edit",
            kind: IntentKind::WorktreeWrite,
            paths: &paths,
        };
        let primary = decide_tool(&intent, &facts("/proj", &adopted, &folders));
        assert_eq!(layer(&primary), Some(DenyLayer::WriteGuard), "{primary:?}");
        let own = [std::path::PathBuf::from("/proj")];
        assert_eq!(
            decide_tool(&intent, &facts("/proj/.claude/worktrees/x", &own, &folders)),
            PolicyDecision::Allow
        );
    }

    #[test]
    fn dot_dot_and_symlinks_cant_smuggle_a_path_into_another_streams_tree() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let own = base.join("proj");
        let other = base.join("proj-wt");
        std::fs::create_dir_all(own.join("src")).unwrap();
        std::fs::create_dir_all(other.join("src")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&other, own.join("lnk")).unwrap();
        let mut paths = vec![format!("{}/../proj-wt/src/a.rs", own.display())];
        #[cfg(unix)]
        paths.push(format!("{}/lnk/src/new.rs", own.display()));
        for p in paths {
            let one = vec![p.clone()];
            let d = decide_tool(
                &ToolIntent {
                    label: "Write",
                    kind: IntentKind::WorktreeWrite,
                    paths: &one,
                },
                &PolicyFacts {
                    worktree_root: &own,
                    other_roots: std::slice::from_ref(&other),
                    project_dir: &own,
                    record_folders: &[],
                },
            );
            assert_eq!(layer(&d), Some(DenyLayer::WriteGuard), "{p}");
        }
    }
}
