//! Typed URI resolver.
//!
//! Hydrates a URI — what the `page_ref` graph, tabs and comment targets
//! hold — into a small, human/LLM-readable [`RefSummary`]. A comment
//! carries its target plus a `context_chain` and `referenced` URIs;
//! before the agent answers a comment we resolve each into a title +
//! detail + body excerpt so the agent sees *what the highlighted thing
//! is* (the commit subject, the task title+status, the wiki lead) without
//! a second tool round-trip. Lives in `oxplow-app` because it reaches
//! across the store/VCS surfaces held by [`Services`].

use serde::{Deserialize, Serialize};

use oxplow_domain::uri::resources as r;
use oxplow_domain::uri::OxplowUri;

use crate::Services;

/// A resolved view of one URI. `title`/`detail`/`body_excerpt` are all
/// best-effort: when the thing is gone (deleted task, unknown sha) or
/// carries no first-class label (a bare file path is its own display)
/// they stay `None` and the caller falls back to the URI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RefSummary {
    pub uri: String,
    /// Its resource (`oxplow/work_item`); empty when it isn't a URI.
    pub resource: String,
    /// Primary human label — wiki/task title, commit subject.
    pub title: Option<String>,
    /// Short secondary fact — task status, commit diffstat.
    pub detail: Option<String>,
    /// A lead excerpt of the body — task description, commit message
    /// body, wiki lead — capped so it stays cheap in the agent's context.
    pub body_excerpt: Option<String>,
    /// Whether anything resolved it. A URI whose resource or row is gone
    /// — an uninstalled extension's, a deleted record's — is a dead link:
    /// shown as its raw URI, never an error.
    pub resolved: bool,
}

impl RefSummary {
    /// An unresolved summary carrying only the URI: the base every
    /// resolver enriches.
    fn bare(uri: &str) -> Self {
        Self {
            uri: uri.to_string(),
            resource: oxplow_domain::uri::build::resource_of(uri).unwrap_or_default(),
            title: None,
            detail: None,
            body_excerpt: None,
            resolved: false,
        }
    }
}

/// Max length of a `body_excerpt`, in chars. Big enough to convey the
/// gist, small enough not to bloat the agent's context.
const EXCERPT_LEN: usize = 280;

/// Resolve one URI into a [`RefSummary`]. Work items, commits, files,
/// directories, wiki pages and findings have richer resolvers; any other
/// resource resolves through the view its declaration names (`resolve:`
/// and `title:`), by its `ref`. Anything else — not a URI, an undeclared
/// resource, a row that's gone — comes back `resolved: false`.
pub async fn resolve_ref(services: &Services, uri: &str) -> RefSummary {
    let base = RefSummary::bare(uri);
    let Ok(parsed) = OxplowUri::parse(uri) else {
        return base;
    };
    let param = |name: &str| parsed.param(name).unwrap_or_default().to_string();
    let mut summary = match base.resource.as_str() {
        r::WORK_ITEM => resolve_work_item(services, base, &parsed.identity()).await,
        r::COMMIT => resolve_commit(services, base, &param("sha")).await,
        r::FILE => resolve_file(services, base, &param("path")).await,
        r::DIR => resolve_directory(services, base, &param("path")).await,
        r::WIKI => resolve_wiki(services, base, &param("slug")).await,
        r::FINDING => resolve_finding(services, base, &param("id")).await,
        _ => resolve_owned(services, base, &parsed).await,
    };
    summary.resolved = summary.title.is_some() || summary.detail.is_some();
    summary
}

/// A resource some dataset holds: its title, by its URI.
async fn resolve_owned(
    services: &Services,
    mut summary: RefSummary,
    uri: &OxplowUri,
) -> RefSummary {
    let vocabulary = services.vocabulary.current();
    let Some(decl) = vocabulary.resources.get(uri.resource()) else {
        return summary;
    };
    let Some(view) = decl.resolve.clone() else {
        return summary;
    };
    let title = decl.title.clone().unwrap_or_else(|| "title".to_string());
    let found = services
        .sql
        .query_sql(
            &format!(
                "SELECT \"{}\" FROM \"{}\" WHERE uri = ?1 LIMIT 1",
                title.replace('"', ""),
                view.replace('"', "")
            ),
            vec![oxplow_db::SqlCell::Text(uri.identity())],
            None,
        )
        .await
        .ok()
        .and_then(|out| out.rows.into_iter().next());
    summary.title = found.as_ref().and_then(|row| match &row[0] {
        oxplow_db::SqlCell::Text(t) => Some(t.clone()),
        oxplow_db::SqlCell::Int(i) => Some(i.to_string()),
        _ => None,
    });
    summary
}

/// Resolve a batch of URIs in order (a comment's `context_chain` and
/// `referenced`).
pub async fn resolve_refs(services: &Services, uris: &[String]) -> Vec<RefSummary> {
    let mut out = Vec::with_capacity(uris.len());
    for uri in uris {
        out.push(resolve_ref(services, uri).await);
    }
    out
}

/// A work item, through the interface: the active list's items only,
/// whichever list it is; another stays bare.
async fn resolve_work_item(services: &Services, mut summary: RefSummary, uri: &str) -> RefSummary {
    let row = services
        .sql
        .query_sql(
            "SELECT title, state, body FROM v_work_item WHERE uri = ?1",
            vec![oxplow_db::SqlCell::Text(uri.to_string())],
            None,
        )
        .await
        .ok()
        .and_then(|out| out.rows.into_iter().next());
    if let Some(row) = row {
        let text = |i: usize| match &row[i] {
            oxplow_db::SqlCell::Text(t) => Some(t.clone()),
            _ => None,
        };
        summary.title = text(0);
        summary.detail = text(1);
        summary.body_excerpt = text(2).as_deref().and_then(excerpt);
    }
    summary
}

async fn resolve_commit(services: &Services, mut summary: RefSummary, sha: &str) -> RefSummary {
    // Resolve against the primary worktree; `revision` accepts both full
    // and short shas.
    let ws = services.worktrees.project_dir();
    if let Ok(Some(detail)) = services.vcs.revision(ws, sha).await {
        summary.title = Some(detail.info.subject);
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

async fn resolve_file(services: &Services, mut summary: RefSummary, path: &str) -> RefSummary {
    let path = services.layout.project_dir.join(path);
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
        // Title stays None: the path is the file's own display.
        summary.detail = Some(human_size(size));
        summary.body_excerpt = excerpt(&head);
    }
    summary
}

async fn resolve_directory(services: &Services, mut summary: RefSummary, path: &str) -> RefSummary {
    let path = services.layout.project_dir.join(path);
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

async fn resolve_wiki(services: &Services, mut summary: RefSummary, slug: &str) -> RefSummary {
    if let Ok(Some(page)) = services.wiki_page_store.get(slug).await {
        summary.title = Some(page.title);
        // body_excerpt is already a stored lead; re-cap defensively so a
        // long stored excerpt can't blow the agent's context budget.
        summary.body_excerpt = excerpt(&page.body_excerpt);
    }
    summary
}

async fn resolve_finding(services: &Services, mut summary: RefSummary, id: &str) -> RefSummary {
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
    use crate::Services;
    use oxplow_domain::uri::build;

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
        let services = Services::in_memory(dir.path()).await.unwrap();
        let task = crate::test_fixtures::file_item(&services, serde_json::json!({ "title": "Fix the flaky test", "body": "It fails on CI only.\nSecond line.", "state": "in_progress" })).await;

        let summary = resolve_ref(
            &services,
            &build::work_item_uri("oxplow", &task.to_string()),
        )
        .await;
        assert_eq!(summary.resource, "oxplow/work_item");
        assert_eq!(summary.title.as_deref(), Some("Fix the flaky test"));
        assert_eq!(summary.detail.as_deref(), Some("in_progress"));
        assert!(summary
            .body_excerpt
            .as_deref()
            .unwrap()
            .starts_with("It fails on CI only."));
    }

    /// A work item resolves through the interface: the active list's
    /// items only — with none, even oxplow's task is bare.
    #[tokio::test]
    async fn a_work_item_resolves_from_the_active_list_only() {
        let dir = git_repo_with_commit("init", "");
        let services = Services::in_memory(dir.path()).await.unwrap();
        let task = crate::test_fixtures::file_item(
            &services,
            serde_json::json!({ "title": "Fix the flaky test" }),
        )
        .await;
        services
            .config
            .write()
            .unwrap()
            .personal_active_providers
            .insert("work_items".into(), "none".into());
        let config = crate::config_service::read_config(&services.config);
        services
            .capabilities
            .publish(&config, &services.db)
            .await
            .unwrap();
        let summary = resolve_ref(
            &services,
            &build::work_item_uri("oxplow", &task.to_string()),
        )
        .await;
        assert_eq!(summary.title, None);
    }

    #[tokio::test]
    async fn unknown_task_id_is_bare() {
        let dir = git_repo_with_commit("init", "");
        let services = Services::in_memory(dir.path()).await.unwrap();
        let summary = resolve_ref(&services, &build::work_item_uri("oxplow", "tsk999999")).await;
        assert_eq!(summary.title, None);
        assert_eq!(summary.detail, None);
        assert_eq!(summary.uri, build::work_item_uri("oxplow", "tsk999999"));
    }

    #[tokio::test]
    async fn resolves_commit_subject_and_stats() {
        let dir = git_repo_with_commit("Add the widget", "Longer rationale here.");
        let services = Services::in_memory(dir.path()).await.unwrap();
        let sha = head_sha(dir.path());

        let summary = resolve_ref(&services, &build::commit_uri(&sha)).await;
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

    /// An old ref is no URI: a caller still saying `oxplow:oxplow/commit?sha=<sha>` or
    /// `oxplow:oxplow/work_item?id=tsk1&provider=oxplow` gets a bare summary.
    #[tokio::test]
    async fn old_refs_are_not_resolved() {
        let dir = git_repo_with_commit("Add the widget", "");
        let services = Services::in_memory(dir.path()).await.unwrap();
        let sha = head_sha(dir.path());
        for old in [
            format!("commit:{sha}"),
            "work_item:oxplow:tsk1".into(),
            "dir:.".into(),
        ] {
            let summary = resolve_ref(&services, &old).await;
            assert_eq!(summary.title, None, "{old} should be bare");
            assert_eq!(summary.resource, "");
        }
    }

    /// Any resource a dataset resolves (its `resolve:`) resolves through
    /// that dataset's `ref` and title; a URI nothing resolves — its
    /// resource undeclared, its row gone — is a dead link, not an error.
    #[tokio::test]
    async fn a_resource_resolves_through_its_dataset_or_is_unresolved() {
        let fx = crate::test_fixtures::services_with_task_effort().await;
        let summary = resolve_ref(&fx.svc, &build::effort_uri(fx.effort)).await;
        assert!(summary.resolved, "{summary:?}");
        assert!(summary.title.is_some(), "{summary:?}");

        for uri in [
            build::keyed(oxplow_domain::uri::resources::EFFORT, "eff999"),
            build::collector_uri("gone", "x"),
            "oxplow:mystery/thing?id=x".to_string(),
        ] {
            let summary = resolve_ref(&fx.svc, &uri).await;
            assert!(!summary.resolved, "{uri} {summary:?}");
            assert_eq!(summary.title, None);
        }
    }

    #[tokio::test]
    async fn an_undeclared_resource_is_bare() {
        let dir = git_repo_with_commit("init", "");
        let services = Services::in_memory(dir.path()).await.unwrap();
        let summary = resolve_ref(&services, "oxplow:mystery/thing?id=whatever").await;
        assert_eq!(summary.resource, "mystery/thing");
        assert_eq!(summary.uri, "oxplow:mystery/thing?id=whatever");
        assert_eq!(summary.title, None);
        assert_eq!(summary.detail, None);
    }

    #[tokio::test]
    async fn resolves_file_size_and_head() {
        let dir = git_repo_with_commit("init", "");
        let services = Services::in_memory(dir.path()).await.unwrap();
        // a.rs is `fn main() {}\n` (13 bytes) created by the fixture.
        let summary = resolve_ref(&services, &build::file_uri("a.rs")).await;
        assert_eq!(summary.resource, "oxplow/file");
        // Path is its own display; no synthetic title.
        assert_eq!(summary.title, None);
        assert_eq!(summary.detail.as_deref(), Some("13 B"));
        assert_eq!(summary.body_excerpt.as_deref(), Some("fn main() {}"));
    }

    #[tokio::test]
    async fn missing_file_is_bare() {
        let dir = git_repo_with_commit("init", "");
        let services = Services::in_memory(dir.path()).await.unwrap();
        let summary = resolve_ref(&services, &build::file_uri("does/not/exist.rs")).await;
        assert_eq!(summary.detail, None);
        assert_eq!(summary.body_excerpt, None);
    }

    #[tokio::test]
    async fn resolves_directory_entry_count_and_names() {
        let dir = git_repo_with_commit("init", "");
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/x.rs"), "x").unwrap();
        std::fs::write(dir.path().join("sub/y.rs"), "y").unwrap();
        let services = Services::in_memory(dir.path()).await.unwrap();
        let summary = resolve_ref(&services, &build::dir_uri("sub")).await;
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
        let services = Services::in_memory(dir.path()).await.unwrap();
        let now = oxplow_domain::Timestamp::now();
        let page = WikiPage {
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
        };
        services
            .db
            .transaction(move |tx| oxplow_db::wiki_page_store::upsert_tx(tx, &page, "", ""))
            .await
            .unwrap();
        let summary = resolve_ref(&services, &build::wiki_uri("architecture")).await;
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
        let services = Services::in_memory(dir.path()).await.unwrap();
        let scan = services
            .code_quality_store
            .create_scan("complexity", "all", "working", "all")
            .await
            .unwrap();
        services
            .code_quality_store
            .finish_scan_with_findings(
                scan,
                vec![CodeQualityFinding {
                    id: 0,
                    scan_id: scan,
                    path: "src/app.rs".into(),
                    start_line: 10,
                    end_line: 42,
                    kind: "high_complexity".into(),
                    metric_value: 17.0,
                    extra_json: None,
                }],
            )
            .await
            .unwrap();
        // The first finding gets id 1 (autoincrement).
        let summary = resolve_ref(&services, &build::finding_uri("1")).await;
        assert_eq!(summary.title.as_deref(), Some("high_complexity"));
        assert_eq!(summary.detail.as_deref(), Some("src/app.rs:10-42"));
    }

    #[tokio::test]
    async fn resolve_refs_preserves_order() {
        let dir = git_repo_with_commit("init", "");
        let services = Services::in_memory(dir.path()).await.unwrap();
        let refs = vec![build::file_uri("a.rs"), build::dir_uri("src")];
        let out = resolve_refs(&services, &refs).await;
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].uri, refs[0]);
        assert_eq!(out[1].uri, refs[1]);
    }
}
