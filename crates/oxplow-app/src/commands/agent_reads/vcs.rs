//! `vcs.read` operations: the repository read through its provider —
//! what changed between two versions of the tree (`oxplow.vcs.diff`) and
//! who last changed each line of a file (`oxplow.vcs.blame`) — and its
//! history (`oxplow.vcs.log`, paged, from any revision, any branch), the
//! changed files (`oxplow.vcs.status`) and the branches
//! (`oxplow.vcs.branches`).

use std::sync::Arc;

use oxplow_domain::vcs::Revision;
use oxplow_domain::CommandError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::super::ops::Op;
use super::super::util::{invalid, parse, schema, ServicesSlot};
use super::super::{Handler, Invocation};
use super::ids::{check_optional_stream, read_op, result_of};
use super::stream_or_actors;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DiffParams {
    /// The stream whose workspace to diff; the caller's when absent.
    pub stream_id: Option<String>,
    /// The older side: `working`, `snap:<id>` or `git:<rev>`; absent =
    /// the empty tree (everything added).
    pub from: Option<String>,
    /// The newer side (e.g. `working`).
    pub to: String,
    /// Compare `to` against where it forked from `from` rather than
    /// against `from` itself — a branch's own changes against `git:main`.
    #[serde(default)]
    pub since_fork: bool,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct BlameParams {
    /// The stream whose workspace to blame in; the caller's when absent.
    pub stream_id: Option<String>,
    /// Workspace-relative file path.
    pub path: String,
    /// Which version: `working` (default; uncommitted lines name no
    /// revision) or `git:<rev>`.
    pub revision: Option<String>,
}

/// How many revisions a log page holds unless the call says.
const LOG_DEFAULT: u32 = 50;
/// The most a log page holds.
const LOG_MAX: u32 = 500;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogParams {
    /// The stream whose repository to read; the caller's when absent. A
    /// stream that doesn't exist is refused.
    pub stream_id: Option<String>,
    /// Where the history starts: any revision — a branch, a tag, an id.
    /// The workspace's head when absent.
    pub from: Option<String>,
    /// Only the revisions that changed this workspace-relative path.
    pub path: Option<String>,
    /// Revisions in the page (default 50, at most 500).
    #[schemars(range(min = 1, max = 500))]
    pub limit: Option<u32>,
    /// The previous page's `next_cursor`: the page resumes after it.
    pub cursor: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StreamParams {
    /// The stream whose repository to read; the caller's when absent. A
    /// stream that doesn't exist is refused.
    pub stream_id: Option<String>,
}

/// One page of history, newest first.
#[derive(Debug, Serialize)]
struct LogPage {
    revisions: Vec<oxplow_domain::vcs::RevisionInfo>,
    /// Pass as `cursor` for the next page; `null` on the last.
    next_cursor: Option<String>,
}

/// The stream a call reads: the one it names (which must exist), else the
/// actor's own.
async fn stream_of(
    svc: &crate::Services,
    actor: &oxplow_domain::Actor,
    tool: &str,
    given: Option<String>,
) -> Result<Option<String>, CommandError> {
    if let Some(named) = &given {
        check_optional_stream(tool, "/stream_id", Some(named))?;
        svc.worktrees
            .resolve_strict(Some(named))
            .await
            .map_err(|e| invalid("/stream_id", e.to_string()))?;
    }
    stream_or_actors(svc, actor, given).await
}

/// A `Revision` a call was given (`working`, `snap:<id>`, `git:<rev>`).
fn parse_revision(field: &str, raw: &str) -> Result<Revision, CommandError> {
    raw.parse().map_err(|e: String| invalid(field, e))
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    let diff_services = services.clone();
    let log_services = services.clone();
    let status_services = services.clone();
    let branches_services = services.clone();
    vec![
        read_op(
            "vcs.read",
            "log",
            log_services,
            |svc, actor, p: LogParams| async move {
                let stream = stream_of(&svc, &actor, "vcs_log", p.stream_id).await?;
                let limit = p.limit.unwrap_or(LOG_DEFAULT).clamp(1, LOG_MAX);
                // One more than a page says whether there is a next.
                let query = oxplow_domain::vcs::LogQuery {
                    limit: Some(limit + 1 + u32::from(p.cursor.is_some())),
                    all: false,
                    from: p.from,
                    path: p.path,
                    after: p.cursor,
                };
                let mut revisions = crate::vcs::reads::log(&svc, stream.as_deref(), query).await?;
                let more = revisions.len() > limit as usize;
                revisions.truncate(limit as usize);
                let next_cursor = more
                    .then(|| revisions.last().map(|r| r.id.clone()))
                    .flatten();
                result_of(LogPage {
                    revisions,
                    next_cursor,
                })
            },
        ),
        read_op(
            "vcs.read",
            "status",
            status_services,
            |svc, actor, p: StreamParams| async move {
                let stream = stream_of(&svc, &actor, "vcs_status", p.stream_id).await?;
                result_of(crate::vcs::reads::status(&svc, stream.as_deref()).await?)
            },
        ),
        read_op(
            "vcs.read",
            "branches",
            branches_services,
            |svc, actor, p: StreamParams| async move {
                let stream = stream_of(&svc, &actor, "vcs_branches", p.stream_id).await?;
                result_of(crate::vcs::reads::branches(&svc, stream.as_deref()).await?)
            },
        ),
        Op::new(
            "vcs.read",
            "diff",
            schema::<DiffParams>(),
            false,
            Handler::External(Arc::new(move |invocation: Invocation, input| {
                let services = diff_services.clone();
                Box::pin(async move {
                    let mut p: DiffParams = parse(input)?;
                    let svc = services.get()?;
                    check_optional_stream("oxplow_diff", "/stream_id", p.stream_id.as_deref())?;
                    let sid =
                        stream_or_actors(&svc, invocation.actor(), p.stream_id.take()).await?;
                    let to = parse_revision("/to", &p.to)?;
                    let mut from = p
                        .from
                        .as_deref()
                        .map(|f| parse_revision("/from", f))
                        .transpose()?;
                    let sid = sid.as_deref();
                    if p.since_fork {
                        let Some(base) = &from else {
                            return Err(invalid("/since_fork", "since_fork needs `from`"));
                        };
                        let head = Revision::from_rev_slot(Some("git:HEAD"))
                            .map_err(|e| invalid("/to", e))?;
                        let fork_of = if to == Revision::Working { &head } else { &to };
                        from = crate::vcs::reads::merge_base(&svc, sid, base, fork_of).await?;
                    }
                    let ws = svc.worktrees.resolve(sid).await.into_local_path();
                    super::ids::result_of(svc.trees.diff(&ws, from.as_ref(), &to).await?)
                })
            })),
        ),
        Op::new(
            "vcs.read",
            "blame",
            schema::<BlameParams>(),
            false,
            Handler::External(Arc::new(move |invocation: Invocation, input| {
                let services = services.clone();
                Box::pin(async move {
                    let p: BlameParams = parse(input)?;
                    let svc = services.get()?;
                    check_optional_stream(
                        "oxplow_vcs_blame",
                        "/stream_id",
                        p.stream_id.as_deref(),
                    )?;
                    let sid = stream_or_actors(&svc, invocation.actor(), p.stream_id).await?;
                    let revision =
                        parse_revision("/revision", p.revision.as_deref().unwrap_or("working"))?;
                    super::ids::result_of(
                        crate::vcs::reads::blame(&svc, sid.as_deref(), &p.path, &revision).await?,
                    )
                })
            })),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::Actor;
    use serde_json::json;

    fn anonymous() -> Actor {
        Actor::Agent {
            session_id: None,
            thread_id: None,
            stream_id: None,
        }
    }

    /// Blame reads through the Vcs provider: each line of the working copy
    /// names the commit that last changed it, an uncommitted line none;
    /// diff lists the changed files against the empty tree or a revision.
    #[tokio::test]
    async fn blame_and_diff_read_the_repository_through_the_bus() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let svc = crate::Services::in_memory(dir.path()).await.unwrap();
        std::fs::write(dir.path().join("b.txt"), "one\ntwo\n").unwrap();
        let repo = git2::Repository::open(dir.path()).unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("b.txt")).unwrap();
        idx.write().unwrap();
        let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        let sig = repo.signature().unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        let commit = repo
            .commit(Some("HEAD"), &sig, &sig, "add b", &tree, &[&parent])
            .unwrap();
        std::fs::write(dir.path().join("b.txt"), "one\ntwo\nthree\n").unwrap();

        let lines = svc
            .commands
            .run(
                &anonymous(),
                "oxplow.vcs.blame",
                json!({ "path": "b.txt" }),
                false,
            )
            .await
            .unwrap()
            .result;
        let lines = lines.as_array().unwrap();
        assert_eq!(lines.len(), 3, "{lines:?}");
        assert!(lines[0].to_string().contains(&commit.to_string()));
        assert!(!lines[2].to_string().contains(&commit.to_string()));

        let diff = svc
            .commands
            .run(
                &anonymous(),
                "oxplow.vcs.diff",
                json!({ "from": "git:HEAD", "to": "working" }),
                false,
            )
            .await
            .unwrap()
            .result;
        assert!(diff.to_string().contains("b.txt"), "{diff}");

        let err = svc
            .commands
            .run(
                &anonymous(),
                "oxplow.vcs.diff",
                json!({ "to": "nope" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Invalid { .. }), "{err:?}");
    }

    fn commit_file(dir: &std::path::Path, path: &str, body: &str, message: &str) -> String {
        std::fs::write(dir.join(path), body).unwrap();
        let repo = git2::Repository::open(dir).unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new(path)).unwrap();
        idx.write().unwrap();
        let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        let sig = repo.signature().unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
            .unwrap()
            .to_string()
    }

    /// History is read live from the provider, paged by the last id and
    /// from any revision; the status lists the changed files; the
    /// branches list names the default; a named stream that isn't one is
    /// refused rather than read from the primary.
    #[tokio::test]
    async fn log_status_and_branches_read_the_repository_through_the_bus() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let svc = crate::Services::in_memory(dir.path()).await.unwrap();
        let one = commit_file(dir.path(), "a.txt", "1\n", "one");
        let two = commit_file(dir.path(), "b.txt", "1\n", "two");
        let three = commit_file(dir.path(), "a.txt", "2\n", "three");
        let run = |id: &'static str, input: serde_json::Value| {
            let svc = svc.clone();
            async move { svc.commands.run(&anonymous(), id, input, false).await }
        };

        let page = run("oxplow.vcs.log", json!({ "limit": 2 }))
            .await
            .unwrap()
            .result;
        let ids: Vec<&str> = page["revisions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, [three.as_str(), two.as_str()]);
        assert_eq!(page["next_cursor"], json!(two));
        let rest = run("oxplow.vcs.log", json!({ "limit": 2, "cursor": two }))
            .await
            .unwrap()
            .result;
        assert_eq!(rest["revisions"][0]["id"], json!(one));
        assert_eq!(
            rest["next_cursor"],
            json!(null),
            "the last page has no cursor"
        );

        let touching = run("oxplow.vcs.log", json!({ "path": "b.txt" }))
            .await
            .unwrap()
            .result;
        assert_eq!(touching["revisions"].as_array().unwrap().len(), 1);
        let from = run("oxplow.vcs.log", json!({ "from": two, "limit": 1 }))
            .await
            .unwrap()
            .result;
        assert_eq!(from["revisions"][0]["id"], json!(two));

        let err = run("oxplow.vcs.log", json!({ "from": "no-such-rev" }))
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                CommandError::Invalid { .. } | CommandError::Failed { .. }
            ),
            "{err:?}"
        );
        let err = run("oxplow.vcs.log", json!({ "limit": 501 }))
            .await
            .unwrap_err();
        assert!(matches!(&err, CommandError::Invalid { .. }), "{err:?}");
        let err = run("oxplow.vcs.log", json!({ "stream_id": "str999" }))
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { message, .. } if message.contains("no stream")),
            "{err:?}"
        );

        std::fs::write(dir.path().join("a.txt"), "3\n").unwrap();
        let status = run("oxplow.vcs.status", json!({})).await.unwrap().result;
        assert!(status["entries"].to_string().contains("a.txt"), "{status}");
        let branches = run("oxplow.vcs.branches", json!({})).await.unwrap().result;
        assert!(
            branches
                .as_array()
                .unwrap()
                .iter()
                .any(|b| b["is_default"] == json!(true)),
            "{branches}"
        );
    }
}
