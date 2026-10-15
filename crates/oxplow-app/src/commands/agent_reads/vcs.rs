//! `vcs.read` operations: the repository read through its provider —
//! what changed between two versions of the tree (`oxplow.vcs.diff`) and
//! who last changed each line of a file (`oxplow.vcs.blame`).

use std::sync::Arc;

use oxplow_domain::vcs::Revision;
use oxplow_domain::CommandError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::super::ops::Op;
use super::super::util::{invalid, parse, schema, ServicesSlot};
use super::super::{Handler, Invocation};
use super::ids::check_optional_stream;
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

/// A `Revision` a call was given (`working`, `snap:<id>`, `git:<rev>`).
fn parse_revision(field: &str, raw: &str) -> Result<Revision, CommandError> {
    raw.parse().map_err(|e: String| invalid(field, e))
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    let diff_services = services.clone();
    vec![
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
                    check_optional_stream("diff", "/stream_id", p.stream_id.as_deref())?;
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
                    check_optional_stream("vcs_blame", "/stream_id", p.stream_id.as_deref())?;
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
}
