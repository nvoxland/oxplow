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
    /// The ref kinds links may name (the running vocabulary's).
    pub kinds: &'a oxplow_domain::refs::kind::KindRegistry,
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
    for link in classify_wikilinks(world.kinds, body) {
        match (&link.reference, &link.canonical) {
            // A kind the vocabulary knows with no typed probe here: a
            // plugin kind is checked against its `resolve` model, any
            // other is a link as it stands (tsk894).
            (None, Some(canonical)) => {
                if let Some(reason) = unresolved_plugin_ref(world, canonical) {
                    out.push(LinkWarning {
                        target: link.raw.clone(),
                        reason,
                    });
                }
            }
            (None, None) => out.push(LinkWarning {
                target: link.raw.clone(),
                reason: format!(
                    "`[[{}]]` is not a recognized reference — use `[[tsk42]]` for a task \
                     (never the GitHub `#42` form), `[[some-slug]]` for a wiki page, \
                     `[[path/to/file.rs]]` for a file, or `[[git:<sha>]]` for a commit",
                    link.raw
                ),
            }),
            (Some(reference), _) => {
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

/// What a command checks a body's links against: the project and its VCS
/// (the database and vocabulary come with its transaction). Notes and
/// oxplow's work items carry one.
#[derive(Clone)]
pub struct LinkDeps {
    pub project_dir: std::path::PathBuf,
    pub vcs: std::sync::Arc<dyn oxplow_domain::vcs::Vcs>,
}

impl LinkDeps {
    /// The links in `body` that don't resolve, checked in the command's
    /// transaction — files in `thread`'s worktree (tsk895), the primary
    /// checkout when it has none.
    pub fn warnings(
        &self,
        ctx: &crate::commands::TxCtx<'_>,
        body: &str,
        thread: Option<oxplow_domain::ThreadId>,
    ) -> Vec<LinkWarning> {
        let root = thread
            .and_then(|t| worktree_of_tx(ctx.conn, t))
            .unwrap_or_else(|| self.project_dir.clone());
        let graph = self.vcs.revision_graph(&root);
        check_links_in(
            &LinkWorld {
                conn: ctx.conn,
                kinds: &ctx.events.vocabulary.kinds,
                project_dir: &root,
                graph: &*graph,
                this_page: None,
            },
            body,
        )
    }
}

/// `thread`'s stream's worktree, when it has one on disk.
pub(crate) fn worktree_of_tx(
    conn: &rusqlite::Connection,
    thread: oxplow_domain::ThreadId,
) -> Option<std::path::PathBuf> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT s.worktree_path FROM threads t JOIN streams s ON s.id = t.stream_id
          WHERE t.id = ?1",
        [thread.value()],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
    .map(std::path::PathBuf::from)
    .filter(|p| p.is_dir())
}

/// [`check_links_in`] over the app's services, for the tools that report
/// warnings rather than refuse (task and note bodies).
pub async fn check_links(services: &Services, body: &str) -> Vec<LinkWarning> {
    check_links_at(
        &services.db,
        &services.vocabulary,
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
    vocabulary: &oxplow_domain::vocabulary::VocabularyHandle,
    project_dir: &std::path::Path,
    vcs: &dyn oxplow_domain::vcs::Vcs,
    body: &str,
) -> Vec<LinkWarning> {
    let project_dir = project_dir.to_path_buf();
    let graph = vcs.revision_graph(&project_dir);
    let body = body.to_string();
    let vocabulary = vocabulary.current();
    db.read(move |conn| {
        Ok(check_links_in(
            &LinkWorld {
                conn,
                kinds: &vocabulary.kinds,
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

/// `Some(reason)` when `canonical` is of a plugin kind whose `resolve`
/// model has no row for it (`v_ref_kind.resolve`, by its `ref` column).
/// A kind with no `resolve` model, or one that can't be read, isn't
/// probed.
fn unresolved_plugin_ref(
    world: &LinkWorld<'_>,
    canonical: &oxplow_domain::refs::grammar::CanonicalRef,
) -> Option<String> {
    use rusqlite::OptionalExtension;
    let resolve: String = world
        .conn
        .query_row(
            "SELECT resolve FROM v_ref_kind WHERE kind = ?1 AND resolve IS NOT NULL",
            [&canonical.kind],
            |r| r.get(0),
        )
        .optional()
        .ok()
        .flatten()?;
    let at = canonical.to_string();
    let found = world
        .conn
        .query_row(
            &format!(
                "SELECT 1 FROM \"{}\" WHERE ref = ?1",
                resolve.replace('"', "")
            ),
            [&at],
            |_| Ok(()),
        )
        .optional()
        .ok()?;
    found
        .is_none()
        .then(|| format!("`{at}` does not exist (no row in `{resolve}`)"))
}

/// `Some(reason)` when a recognized reference's object doesn't exist,
/// `None` when it resolves.
fn missing_reason(world: &LinkWorld<'_>, reference: &Reference) -> Option<String> {
    match reference {
        // Through the interface: the active list's items.
        Reference::Task(id) => (!exists(
            world.conn,
            "SELECT 1 FROM v_work_item WHERE ref = 'work_item:oxplow:tsk' || ?1",
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
        // A file at a revision is looked for there (tsk895); one on disk in
        // the worktree.
        Reference::File(detail) => match &detail.version {
            oxplow_domain::refs::RefVersion::Ref(rev) => {
                match world.graph.has_file(rev, &detail.path) {
                    Some(true) => None,
                    Some(false) => {
                        Some(format!("file `{}` does not exist at `{rev}`", detail.path))
                    }
                    None => Some(format!("revision `{rev}` was not found")),
                }
            }
            oxplow_domain::refs::RefVersion::Disk => {
                (!world.project_dir.join(&detail.path).is_file())
                    .then(|| format!("file `{}` does not exist", detail.path))
            }
        },
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

    /// tsk894: a ref of any kind the vocabulary knows is a link — an
    /// effort, another provider's work item — even with no typed view
    /// to probe.
    #[tokio::test]
    async fn accepts_refs_of_kinds_with_no_probe() {
        let dir = git_repo();
        let services = Services::in_memory(dir.path()).unwrap();
        let warnings = check_links(
            &services,
            "See [[effort:eff1]] and [[work_item:issues:ENG-12]].",
        )
        .await;
        assert!(warnings.is_empty(), "got {warnings:?}");
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

    /// A work-item link checks the interface: with no list, an existing
    /// oxplow task is nothing it can point at.
    #[tokio::test]
    async fn a_task_link_checks_the_active_list() {
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
        let warnings = check_links(&services, &format!("See [[{}]].", task.id)).await;
        assert_eq!(warnings.len(), 1, "{warnings:?}");
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

    /// tsk895: a file pinned to a revision is looked for at that revision,
    /// not on disk — one since deleted is still a good link.
    #[tokio::test]
    async fn a_pinned_file_is_checked_at_its_revision() {
        let dir = git_repo();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "fn a() {}\n").unwrap();
        let repo = git2::Repository::open(dir.path()).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(std::path::Path::new("src/a.rs")).unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = repo.signature().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "a", &tree, &[])
            .unwrap();
        std::fs::remove_file(dir.path().join("src/a.rs")).unwrap();
        let services = Services::in_memory(dir.path()).unwrap();
        let warnings =
            check_links(&services, "Was [[src/a.rs@HEAD]], never [[src/b.rs@HEAD]].").await;
        assert_eq!(
            warnings
                .iter()
                .map(|w| w.target.as_str())
                .collect::<Vec<_>>(),
            vec!["src/b.rs@HEAD"],
            "{warnings:?}"
        );
        assert!(warnings[0].reason.contains("at `HEAD`"), "{warnings:?}");
    }
}
