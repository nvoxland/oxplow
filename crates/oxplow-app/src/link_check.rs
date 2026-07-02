//! Wikilink validity checker.
//!
//! Given a body (wiki page, task summary/description, thread note),
//! flags every `[[…]]` wikilink whose target is invalid — either an
//! **unrecognized ref shape** (e.g. `[[#13]]`, the GitHub form) or a
//! **recognized ref whose object doesn't exist** (`[[tsk999]]`,
//! `[[missing-slug]]`, `[[src/gone.rs]]`). MCP write tools surface the
//! result so the authoring agent can self-correct in the same turn.
//!
//! Classification (recognized-or-not) lives in
//! [`oxplow_domain::refs::classify_wikilinks`]; this module adds only the
//! IO-backed existence probes, reusing the same store/git/fs surfaces as
//! [`crate::ref_resolver`].

use serde::{Deserialize, Serialize};

use oxplow_domain::refs::{classify_wikilinks, Reference};

use crate::Services;

/// One invalid wikilink found in a body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkWarning {
    /// The raw `[[…]]` interior as authored (the target, e.g. `#13`,
    /// `tsk999`, `missing-slug`).
    pub target: String,
    /// Why it's invalid — human/LLM-readable, actionable.
    pub reason: String,
}

/// Check every `[[…]]` wikilink in `body`, returning one [`LinkWarning`]
/// per invalid link (empty when every link is valid). Links inside code
/// spans / fenced blocks are ignored (they're illustrative, not refs).
pub async fn check_links(services: &Services, body: &str) -> Vec<LinkWarning> {
    let mut out = Vec::new();
    for link in classify_wikilinks(body) {
        match &link.reference {
            None => out.push(LinkWarning {
                target: link.raw.clone(),
                reason: format!(
                    "`[[{}]]` is not a recognized reference — use `[[tsk42]]` for a task \
                     (never the GitHub `#42` form), `[[some-slug]]` for a wiki page, \
                     `[[path/to/file.rs]]` for a file, or `[[git:<sha>]]` for a commit",
                    link.raw
                ),
            }),
            Some(reference) => {
                if let Some(reason) = missing_reason(services, reference).await {
                    out.push(LinkWarning {
                        target: link.raw.clone(),
                        reason,
                    });
                }
            }
        }
    }
    out
}

/// `Some(reason)` when a recognized reference's object doesn't exist,
/// `None` when it resolves. Mirrors the per-kind lookups in
/// [`crate::ref_resolver`].
async fn missing_reason(services: &Services, reference: &Reference) -> Option<String> {
    match reference {
        Reference::Task(id) => {
            use oxplow_domain::stores::TaskStore as _;
            let tid = oxplow_domain::TaskId::new(*id);
            let exists = matches!(services.task_store.get(tid).await, Ok(Some(_)));
            (!exists).then(|| format!("task tsk{id} does not exist"))
        }
        Reference::Wiki(slug) => {
            let exists = matches!(services.wiki_page_store.get(slug).await, Ok(Some(_)));
            (!exists).then(|| format!("wiki page `{slug}` does not exist"))
        }
        Reference::Commit(sha) => {
            let exists = services
                .git
                .commit_detail(None, sha.clone())
                .await
                .is_some();
            (!exists).then(|| format!("commit `{sha}` was not found"))
        }
        Reference::File(detail) => {
            let path = services.layout.project_dir.join(&detail.path);
            let exists = tokio::task::spawn_blocking(move || path.is_file())
                .await
                .unwrap_or(false);
            (!exists).then(|| format!("file `{}` does not exist", detail.path))
        }
        Reference::Dir(dir) => {
            let path = services.layout.project_dir.join(dir);
            let exists = tokio::task::spawn_blocking(move || path.is_dir())
                .await
                .unwrap_or(false);
            (!exists).then(|| format!("directory `{dir}` does not exist"))
        }
        Reference::Finding(id) => {
            let Ok(fid) = id.parse::<i64>() else {
                return Some(format!("finding `{id}` is not a valid finding id"));
            };
            let exists = matches!(
                services.code_quality_store.get_finding(fid).await,
                Ok(Some(_))
            );
            (!exists).then(|| format!("finding `{id}` does not exist"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CreateTaskInput, Services};

    fn git_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let repo = git2::Repository::init(dir.path()).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        dir
    }

    #[tokio::test]
    async fn flags_unrecognized_github_style_ref() {
        let dir = git_repo();
        let services = Services::in_memory(dir.path()).unwrap();
        let warnings = check_links(&services, "See [[#13]] for the follow-up.").await;
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].target, "#13");
        assert!(warnings[0].reason.contains("not a recognized reference"));
    }

    #[tokio::test]
    async fn flags_nonexistent_task() {
        let dir = git_repo();
        let services = Services::in_memory(dir.path()).unwrap();
        let warnings = check_links(&services, "Blocked by [[tsk999]].").await;
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].target, "tsk999");
        assert!(warnings[0].reason.contains("tsk999 does not exist"));
    }

    #[tokio::test]
    async fn accepts_existing_task() {
        let dir = git_repo();
        let services = Services::in_memory(dir.path()).unwrap();
        let task = services
            .tasks
            .create(
                None,
                CreateTaskInput {
                    title: "Real task".into(),
                    description: None,
                    parent_id: None,
                    status: None,
                    priority: None,
                    author: None,
                },
            )
            .await
            .unwrap();
        let body = format!("Done in [[{}]].", task.id);
        let warnings = check_links(&services, &body).await;
        assert!(warnings.is_empty(), "got {warnings:?}");
    }

    #[tokio::test]
    async fn flags_missing_wiki_slug_but_not_code_fenced() {
        let dir = git_repo();
        let services = Services::in_memory(dir.path()).unwrap();
        // The fenced example must be ignored; only the prose link counts.
        let body = "Prose [[ghost-page]].\n\n```\n[[also-ghost]]\n```\n";
        let warnings = check_links(&services, body).await;
        assert_eq!(warnings.len(), 1, "got {warnings:?}");
        assert_eq!(warnings[0].target, "ghost-page");
        assert!(warnings[0].reason.contains("does not exist"));
    }

    #[tokio::test]
    async fn flags_missing_file() {
        let dir = git_repo();
        let services = Services::in_memory(dir.path()).unwrap();
        let warnings = check_links(&services, "Edited [[src/gone.rs]].").await;
        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].target, "src/gone.rs");
        assert!(warnings[0].reason.contains("does not exist"));
    }
}
