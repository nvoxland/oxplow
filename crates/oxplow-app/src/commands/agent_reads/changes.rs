//! `changes.read/ensure`: analyze a change and return its row
//! (`oxplow.change.ensure`). It fills a derived cache (the change's
//! analysis, recomputed from the VCS on demand) and changes no record,
//! so it is a read.

use oxplow_domain::CommandError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::super::ops::Op;
use super::super::util::{invalid, ServicesSlot};
use super::ids::{read_op, result_of};
use super::stream_or_actors;
use crate::change_analysis::ChangeTarget;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct EnsureChangeParams {
    /// `commit` (a commit vs its parent), `effort` (an effort's start →
    /// end), or `working` (a stream's uncommitted work vs HEAD).
    pub kind: String,
    /// For `commit`: a sha or revspec (`HEAD`, `main~2`).
    pub sha: Option<String>,
    /// For `effort`: the effort id (`eff42`).
    pub effort_id: Option<String>,
    /// For `working` and `commit`: the stream id; your own when omitted.
    pub stream_id: Option<String>,
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    vec![read_op(
        "changes.read",
        "ensure",
        services,
        |svc, actor, p: EnsureChangeParams| async move {
            // Omitted: the caller's own stream.
            let stream = stream_or_actors(&svc, &actor, p.stream_id.clone()).await?;
            let need = |v: Option<String>, name: &str| {
                v.ok_or_else(|| {
                    invalid(
                        &format!("/{name}"),
                        format!("`{name}` is required for kind `{}`", p.kind),
                    )
                })
            };
            let target = match p.kind.as_str() {
                "commit" => ChangeTarget::Commit {
                    sha: need(p.sha.clone(), "sha")?,
                    stream_id: stream.clone(),
                },
                "effort" => ChangeTarget::Effort {
                    effort_id: need(p.effort_id.clone(), "effort_id")?,
                },
                "working" => ChangeTarget::Working {
                    stream_id: need(stream.clone(), "stream_id")?,
                },
                other => {
                    return Err(invalid(
                        "/kind",
                        format!("kind `{other}` isn't commit, effort or working"),
                    ))
                }
            };
            let row = crate::change_analysis::ensure_change(&svc, target)
                .await
                .map_err(|e| CommandError::Invalid {
                    field: None,
                    message: e.to_string(),
                })?;
            result_of(row)
        },
    )]
}

#[cfg(test)]
mod tests {
    use oxplow_domain::{Actor, CommandError};
    use serde_json::json;

    /// A commit is analyzed through the bus (no record left); an unknown
    /// kind is refused, naming it.
    #[tokio::test]
    async fn a_commit_is_analyzed_and_an_unknown_kind_refused() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let svc = crate::Services::in_memory(dir.path()).await.unwrap();
        std::fs::write(dir.path().join("a.rs"), "fn a() {}\n").unwrap();
        let repo = git2::Repository::open(dir.path()).unwrap();
        let mut idx = repo.index().unwrap();
        idx.add_path(std::path::Path::new("a.rs")).unwrap();
        idx.write().unwrap();
        let tree = repo.find_tree(idx.write_tree().unwrap()).unwrap();
        let sig = repo.signature().unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "a", &tree, &[&parent])
            .unwrap();
        let actor = Actor::Agent {
            session_id: None,
            thread_id: None,
            stream_id: None,
        };
        let out = svc
            .commands
            .run(
                &actor,
                "oxplow.change.ensure",
                json!({ "kind": "commit", "sha": "HEAD" }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["status"], "done");
        assert!(out.audit_id.is_none(), "a read leaves no record");
        let err = svc
            .commands
            .run(
                &actor,
                "oxplow.change.ensure",
                json!({ "kind": "sideways" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { message, .. } if message.contains("sideways")),
            "{err:?}"
        );
    }
}
