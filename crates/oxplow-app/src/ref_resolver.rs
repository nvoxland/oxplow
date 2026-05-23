//! Typed reference resolver.
//!
//! Hydrates a canonical `(kind,id)` reference — the same vocabulary the
//! `page_ref` graph, tab ids, and comment targets use — into a small,
//! human/LLM-readable [`RefSummary`]. A comment carries its primary
//! target plus a `context_chain` and `referenced_refs` as bare
//! `(kind,id)` pairs; before the agent answers a comment we resolve each
//! of those into a title + detail + body excerpt so the agent sees *what
//! the highlighted thing is* (the commit subject, the task title+status,
//! the wiki lead) without a second tool round-trip.
//!
//! This generalizes the best-effort `source_label` previously buried in
//! the IPC backlinks reader: that returned only a label string; this
//! returns the richer summary the agent context needs. Lives in
//! `oxplow-app` because it reaches across the store/git surfaces
//! (`task_store`, `wiki_page_store`, `git`) held by [`Services`].

use serde::{Deserialize, Serialize};

use oxplow_domain::comment::CommentTarget;
use oxplow_domain::task::TaskStatus;

use crate::Services;

/// A resolved view of one canonical reference. `title`/`detail`/
/// `body_excerpt` are all best-effort: when the referenced thing is gone
/// (deleted task, unknown sha) or the kind carries no first-class label
/// (a bare file path is its own display) they stay `None` and the caller
/// falls back to `kind`/`id`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefSummary {
    pub kind: String,
    pub id: String,
    /// Primary human label — wiki/task title, commit subject.
    pub title: Option<String>,
    /// Short secondary fact — task status, commit diffstat.
    pub detail: Option<String>,
    /// A lead excerpt of the body — task description, commit message
    /// body, wiki lead — capped so it stays cheap in the agent's context.
    pub body_excerpt: Option<String>,
}

impl RefSummary {
    /// An unresolved summary carrying only the canonical id. Returned
    /// for unknown kinds and as the base every resolver enriches.
    fn bare(kind: &str, id: &str) -> Self {
        Self {
            kind: kind.to_string(),
            id: id.to_string(),
            title: None,
            detail: None,
            body_excerpt: None,
        }
    }
}

/// Max length of a `body_excerpt`, in chars. Big enough to convey the
/// gist, small enough not to bloat the agent's context.
const EXCERPT_LEN: usize = 280;

/// Resolve a single `(kind,id)` reference into a [`RefSummary`].
/// Currently hydrates `task` and `git-commit`; every other kind returns
/// a bare summary (the canonical id is the meaningful display for files,
/// directories, and findings). New kinds slot in as additional arms.
pub async fn resolve_ref(services: &Services, kind: &str, id: &str) -> RefSummary {
    match kind {
        "task" => resolve_task(services, id).await,
        "git-commit" => resolve_commit(services, id).await,
        _ => RefSummary::bare(kind, id),
    }
}

/// Resolve a batch of refs in order (used for a comment's `context_chain`
/// and `referenced_refs`).
pub async fn resolve_refs(services: &Services, refs: &[CommentTarget]) -> Vec<RefSummary> {
    let mut out = Vec::with_capacity(refs.len());
    for r in refs {
        out.push(resolve_ref(services, &r.kind, &r.id).await);
    }
    out
}

async fn resolve_task(services: &Services, id: &str) -> RefSummary {
    use oxplow_domain::stores::TaskStore as _;
    let mut summary = RefSummary::bare("task", id);
    let Some(tid) = oxplow_domain::TaskId::try_from_str(id) else {
        return summary;
    };
    if let Ok(Some(task)) = services.task_store.get(tid).await {
        summary.title = Some(task.title);
        summary.detail = Some(status_label(task.status).to_string());
        summary.body_excerpt = excerpt(&task.description);
    }
    summary
}

async fn resolve_commit(services: &Services, id: &str) -> RefSummary {
    let mut summary = RefSummary::bare("git-commit", id);
    // Resolve against the primary worktree; `commit_detail` accepts both
    // full and short shas.
    if let Some(detail) = services.git.commit_detail(None, id.to_string()).await {
        summary.title = Some(detail.subject);
        let files = detail.files.len();
        let additions: u32 = detail.files.iter().map(|f| f.additions).sum();
        let deletions: u32 = detail.files.iter().map(|f| f.deletions).sum();
        let plural = if files == 1 { "file" } else { "files" };
        summary.detail = Some(format!("{files} {plural}, +{additions} -{deletions}"));
        summary.body_excerpt = excerpt(&detail.body);
    }
    summary
}

/// Snake-case status label matching the wire form (`in_progress`, …).
fn status_label(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Ready => "ready",
        TaskStatus::InProgress => "in_progress",
        TaskStatus::Blocked => "blocked",
        TaskStatus::Done => "done",
        TaskStatus::Canceled => "canceled",
        TaskStatus::Archived => "archived",
    }
}

/// First non-empty trimmed lead of `body`, capped at [`EXCERPT_LEN`]
/// chars (with an ellipsis when truncated). `None` for an empty body.
fn excerpt(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut chars = trimmed.chars();
    let head: String = chars.by_ref().take(EXCERPT_LEN).collect();
    if chars.next().is_some() {
        Some(format!("{head}…"))
    } else {
        Some(head)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CreateTaskInput, Services};

    /// A tempdir holding a real git repo with a single configured commit
    /// touching `a.rs`, so commit resolution has something to find.
    fn git_repo_with_commit(subject: &str, body: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn main() {}\n").unwrap();
        let sig = repo.signature().unwrap();
        let tree_id = {
            let mut idx = repo.index().unwrap();
            idx.add_path(std::path::Path::new("a.rs")).unwrap();
            idx.write_tree().unwrap()
        };
        let tree = repo.find_tree(tree_id).unwrap();
        let msg = if body.is_empty() {
            subject.to_string()
        } else {
            format!("{subject}\n\n{body}")
        };
        repo.commit(Some("HEAD"), &sig, &sig, &msg, &tree, &[])
            .unwrap();
        dir
    }

    fn head_sha(dir: &std::path::Path) -> String {
        let repo = git2::Repository::open(dir).unwrap();
        let commit = repo.head().unwrap().peel_to_commit().unwrap();
        commit.id().to_string()
    }

    #[tokio::test]
    async fn resolves_task_title_and_status() {
        let dir = git_repo_with_commit("init", "");
        let services = Services::in_memory(dir.path()).unwrap();
        let task = services
            .tasks
            .create(
                None,
                CreateTaskInput {
                    title: "Fix the flaky test".into(),
                    description: Some("It fails on CI only.\nSecond line.".into()),
                    parent_id: None,
                    status: Some(oxplow_domain::task::TaskStatus::InProgress),
                    priority: None,
                    author: None,
                },
            )
            .await
            .unwrap();

        let summary = resolve_ref(&services, "task", &task.id.to_string()).await;
        assert_eq!(summary.kind, "task");
        assert_eq!(summary.title.as_deref(), Some("Fix the flaky test"));
        assert_eq!(summary.detail.as_deref(), Some("in_progress"));
        assert!(summary
            .body_excerpt
            .as_deref()
            .unwrap()
            .starts_with("It fails on CI only."));
    }

    #[tokio::test]
    async fn unknown_task_id_is_bare() {
        let dir = git_repo_with_commit("init", "");
        let services = Services::in_memory(dir.path()).unwrap();
        let summary = resolve_ref(&services, "task", "999999").await;
        assert_eq!(summary.title, None);
        assert_eq!(summary.detail, None);
        assert_eq!(summary.id, "999999");
    }

    #[tokio::test]
    async fn resolves_commit_subject_and_stats() {
        let dir = git_repo_with_commit("Add the widget", "Longer rationale here.");
        let services = Services::in_memory(dir.path()).unwrap();
        let sha = head_sha(dir.path());

        let summary = resolve_ref(&services, "git-commit", &sha).await;
        assert_eq!(summary.title.as_deref(), Some("Add the widget"));
        // a.rs is one added file.
        let detail = summary.detail.unwrap();
        assert!(detail.starts_with("1 file,"), "got {detail}");
        assert!(detail.contains("+1"), "got {detail}");
        assert_eq!(
            summary.body_excerpt.as_deref(),
            Some("Longer rationale here.")
        );
    }

    #[tokio::test]
    async fn unknown_kind_is_bare() {
        let dir = git_repo_with_commit("init", "");
        let services = Services::in_memory(dir.path()).unwrap();
        let summary = resolve_ref(&services, "file", "src/app.rs").await;
        assert_eq!(summary.kind, "file");
        assert_eq!(summary.id, "src/app.rs");
        assert_eq!(summary.title, None);
    }

    #[tokio::test]
    async fn resolve_refs_preserves_order() {
        let dir = git_repo_with_commit("init", "");
        let services = Services::in_memory(dir.path()).unwrap();
        let refs = vec![
            CommentTarget {
                kind: "file".into(),
                id: "a.rs".into(),
            },
            CommentTarget {
                kind: "directory".into(),
                id: "src".into(),
            },
        ];
        let out = resolve_refs(&services, &refs).await;
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, "a.rs");
        assert_eq!(out[1].id, "src");
    }
}
