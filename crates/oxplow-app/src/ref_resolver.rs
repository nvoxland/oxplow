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
/// Hydrates the canonical page kinds — `task`, `git-commit`, `file`,
/// `directory`, `wiki`, `finding`; any other kind returns a bare summary
/// (its canonical id is its own meaningful display). New kinds slot in as
/// additional arms.
pub async fn resolve_ref(services: &Services, kind: &str, id: &str) -> RefSummary {
    match kind {
        "task" => resolve_task(services, id).await,
        "git-commit" => resolve_commit(services, id).await,
        "file" => resolve_file(services, id).await,
        "directory" => resolve_directory(services, id).await,
        "wiki" => resolve_wiki(services, id).await,
        "finding" => resolve_finding(services, id).await,
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

/// Bytes of a file head we read for the excerpt + line count. Bounded so
/// resolving a file ref never reads an arbitrarily large blob into memory.
const FILE_HEAD_BYTES: u64 = 8 * 1024;

/// Names listed in a directory's `body_excerpt`, so the agent sees a few
/// representative entries without us serializing a huge tree.
const DIR_NAME_LIMIT: usize = 12;

async fn resolve_file(services: &Services, id: &str) -> RefSummary {
    let mut summary = RefSummary::bare("file", id);
    let path = services.layout.project_dir.join(id);
    let read = tokio::task::spawn_blocking(move || -> Option<(u64, String)> {
        use std::io::Read as _;
        let meta = std::fs::metadata(&path).ok()?;
        if !meta.is_file() {
            return None;
        }
        let file = std::fs::File::open(&path).ok()?;
        let mut buf = Vec::new();
        file.take(FILE_HEAD_BYTES).read_to_end(&mut buf).ok()?;
        Some((meta.len(), String::from_utf8_lossy(&buf).into_owned()))
    })
    .await
    .ok()
    .flatten();
    if let Some((size, head)) = read {
        // Title stays None: the path (id) is the file's own display.
        summary.detail = Some(human_size(size));
        summary.body_excerpt = excerpt(&head);
    }
    summary
}

async fn resolve_directory(services: &Services, id: &str) -> RefSummary {
    let mut summary = RefSummary::bare("directory", id);
    let path = services.layout.project_dir.join(id);
    let listed = tokio::task::spawn_blocking(move || -> Option<(usize, Vec<String>)> {
        let mut names: Vec<String> = Vec::new();
        let mut count = 0usize;
        for entry in std::fs::read_dir(&path).ok()? {
            let Ok(entry) = entry else { continue };
            count += 1;
            if names.len() < DIR_NAME_LIMIT {
                names.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
        Some((count, names))
    })
    .await
    .ok()
    .flatten();
    if let Some((count, mut names)) = listed {
        let plural = if count == 1 { "entry" } else { "entries" };
        summary.detail = Some(format!("{count} {plural}"));
        if !names.is_empty() {
            names.sort();
            summary.body_excerpt = Some(names.join(", "));
        }
    }
    summary
}

async fn resolve_wiki(services: &Services, id: &str) -> RefSummary {
    let mut summary = RefSummary::bare("wiki", id);
    if let Ok(Some(page)) = services.wiki_page_store.get(id).await {
        summary.title = Some(page.title);
        // body_excerpt is already a stored lead; re-cap defensively so a
        // long stored excerpt can't blow the agent's context budget.
        summary.body_excerpt = excerpt(&page.body_excerpt);
    }
    summary
}

async fn resolve_finding(services: &Services, id: &str) -> RefSummary {
    let mut summary = RefSummary::bare("finding", id);
    let Ok(fid) = id.parse::<i64>() else {
        return summary;
    };
    if let Ok(Some(f)) = services.code_quality_store.get_finding(fid).await {
        summary.title = Some(f.kind);
        summary.detail = Some(format!("{}:{}-{}", f.path, f.start_line, f.end_line));
    }
    summary
}

/// Format a byte count as a compact human size (`512 B`, `1.2 KB`, …).
fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    let b = bytes as f64;
    if b < KB {
        format!("{bytes} B")
    } else if b < MB {
        format!("{:.1} KB", b / KB)
    } else {
        format!("{:.1} MB", b / MB)
    }
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
        let summary = resolve_ref(&services, "mystery", "whatever").await;
        assert_eq!(summary.kind, "mystery");
        assert_eq!(summary.id, "whatever");
        assert_eq!(summary.title, None);
        assert_eq!(summary.detail, None);
    }

    #[tokio::test]
    async fn resolves_file_size_and_head() {
        let dir = git_repo_with_commit("init", "");
        let services = Services::in_memory(dir.path()).unwrap();
        // a.rs is `fn main() {}\n` (13 bytes) created by the fixture.
        let summary = resolve_ref(&services, "file", "a.rs").await;
        assert_eq!(summary.kind, "file");
        // Path is its own display; no synthetic title.
        assert_eq!(summary.title, None);
        assert_eq!(summary.detail.as_deref(), Some("13 B"));
        assert_eq!(summary.body_excerpt.as_deref(), Some("fn main() {}"));
    }

    #[tokio::test]
    async fn missing_file_is_bare() {
        let dir = git_repo_with_commit("init", "");
        let services = Services::in_memory(dir.path()).unwrap();
        let summary = resolve_ref(&services, "file", "does/not/exist.rs").await;
        assert_eq!(summary.detail, None);
        assert_eq!(summary.body_excerpt, None);
    }

    #[tokio::test]
    async fn resolves_directory_entry_count_and_names() {
        let dir = git_repo_with_commit("init", "");
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/x.rs"), "x").unwrap();
        std::fs::write(dir.path().join("sub/y.rs"), "y").unwrap();
        let services = Services::in_memory(dir.path()).unwrap();
        let summary = resolve_ref(&services, "directory", "sub").await;
        assert_eq!(summary.detail.as_deref(), Some("2 entries"));
        let names = summary.body_excerpt.unwrap();
        assert!(
            names.contains("x.rs") && names.contains("y.rs"),
            "got {names}"
        );
    }

    #[tokio::test]
    async fn resolves_wiki_title_and_lead() {
        use oxplow_db::wiki_page_store::WikiPage;
        let dir = git_repo_with_commit("init", "");
        let services = Services::in_memory(dir.path()).unwrap();
        let now = oxplow_domain::Timestamp::now();
        services
            .wiki_page_store
            .upsert(&WikiPage {
                slug: "architecture".into(),
                title: "System Architecture".into(),
                body_path: "architecture.md".into(),
                body_excerpt: "The workspace isolation rule.".into(),
                body_size_bytes: 30,
                file_refs: vec![],
                dir_refs: vec![],
                related_notes: vec![],
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        let summary = resolve_ref(&services, "wiki", "architecture").await;
        assert_eq!(summary.title.as_deref(), Some("System Architecture"));
        assert_eq!(
            summary.body_excerpt.as_deref(),
            Some("The workspace isolation rule.")
        );
    }

    #[tokio::test]
    async fn resolves_finding_kind_and_location() {
        use oxplow_db::analytics_stores::CodeQualityFinding;
        let dir = git_repo_with_commit("init", "");
        let services = Services::in_memory(dir.path()).unwrap();
        let scan = services
            .code_quality_store
            .create_scan("complexity", "all")
            .await
            .unwrap();
        services
            .code_quality_store
            .append_finding(
                scan,
                CodeQualityFinding {
                    id: 0,
                    scan_id: scan,
                    path: "src/app.rs".into(),
                    start_line: 10,
                    end_line: 42,
                    kind: "high_complexity".into(),
                    metric_value: 17.0,
                    extra_json: None,
                },
            )
            .await
            .unwrap();
        // The first finding gets id 1 (autoincrement).
        let summary = resolve_ref(&services, "finding", "1").await;
        assert_eq!(summary.title.as_deref(), Some("high_complexity"));
        assert_eq!(summary.detail.as_deref(), Some("src/app.rs:10-42"));
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
