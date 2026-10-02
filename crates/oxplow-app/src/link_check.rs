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
//! existence probes — the database, the project's files and the VCS's
//! revision graph — synchronously, so `knowledge.write_page` refuses a
//! dangling link inside its transaction.

use serde::{Deserialize, Serialize};

use oxplow_domain::refs::{classify_wikilinks, Reference};
use oxplow_domain::vcs::RevisionGraph;

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

/// What a link check looks things up in: the database, the project's
/// files and its revision graph. `this_page` is the wiki page being
/// written, which may link to itself before its row exists.
pub struct LinkWorld<'a> {
    pub conn: &'a rusqlite::Connection,
    pub project_dir: &'a std::path::Path,
    pub graph: &'a dyn RevisionGraph,
    pub this_page: Option<&'a str>,
}

/// Check every `[[…]]` wikilink in `body`, returning one [`LinkWarning`]
/// per invalid link (empty when every link is valid). Links inside code
/// spans / fenced blocks are ignored (they're illustrative, not refs).
/// Synchronous, so a command validates inside its transaction.
pub fn check_links_in(world: &LinkWorld<'_>, body: &str) -> Vec<LinkWarning> {
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
                if let Some(reason) = missing_reason(world, reference) {
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

/// [`check_links_in`] over the app's services, for the tools that report
/// warnings rather than refuse (task and note bodies).
pub async fn check_links(services: &Services, body: &str) -> Vec<LinkWarning> {
    check_links_at(
        &services.db,
        &services.layout.project_dir,
        &*services.vcs,
        body,
    )
    .await
}

/// [`check_links_in`] against a project's database and VCS, for a command
/// run outside the bus's transaction (`effort.report`).
pub async fn check_links_at(
    db: &oxplow_db::Database,
    project_dir: &std::path::Path,
    vcs: &dyn oxplow_domain::vcs::Vcs,
    body: &str,
) -> Vec<LinkWarning> {
    let project_dir = project_dir.to_path_buf();
    let graph = vcs.revision_graph(&project_dir);
    let body = body.to_string();
    db.read(move |conn| {
        Ok(check_links_in(
            &LinkWorld {
                conn,
                project_dir: &project_dir,
                graph: &*graph,
                this_page: None,
            },
            &body,
        ))
    })
    .await
    .unwrap_or_default()
}

fn exists(conn: &rusqlite::Connection, sql: &str, param: &dyn rusqlite::ToSql) -> bool {
    conn.query_row(sql, [param], |_| Ok(()))
        .map(|()| true)
        .unwrap_or(false)
}

/// `Some(reason)` when a recognized reference's object doesn't exist,
/// `None` when it resolves.
fn missing_reason(world: &LinkWorld<'_>, reference: &Reference) -> Option<String> {
    match reference {
        Reference::Task(id) => (!exists(
            world.conn,
            "SELECT 1 FROM task WHERE id = ?1 AND deleted_at IS NULL",
            id,
        ))
        .then(|| format!("task tsk{id} does not exist")),
        Reference::Wiki(slug) => (world.this_page != Some(slug.as_str())
            && !exists(world.conn, "SELECT 1 FROM wiki_page WHERE slug = ?1", slug))
        .then(|| format!("wiki page `{slug}` does not exist")),
        Reference::Commit(sha) => world
            .graph
            .resolve(sha)
            .is_none()
            .then(|| format!("commit `{sha}` was not found")),
        Reference::File(detail) => (!world.project_dir.join(&detail.path).is_file())
            .then(|| format!("file `{}` does not exist", detail.path)),
        Reference::Dir(dir) => (!world.project_dir.join(dir).is_dir())
            .then(|| format!("directory `{dir}` does not exist")),
        Reference::Finding(id) => match id.parse::<i64>() {
            Err(_) => Some(format!("finding `{id}` is not a valid finding id")),
            Ok(fid) => (!exists(
                world.conn,
                "SELECT 1 FROM code_quality_finding WHERE id = ?1",
                &fid,
            ))
            .then(|| format!("finding `{id}` does not exist")),
        },
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
