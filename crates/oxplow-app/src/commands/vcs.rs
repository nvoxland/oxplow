//! `vcs.*` and `git.*`: version-control mutations as commands (P5.B6,
//! `.context/vcs.md`, `.context/commands.md`). Each drives the VCS — a
//! system the bus doesn't own — so each is `External`: it runs, then the
//! bus audits it, the result (the VCS's `OpOutcome`, conflicts included)
//! on the audit row. All are a person's: agents keep no VCS mutations
//! (they run `git` in their own terminal, which oxplow doesn't police).
//! None is undoable. Each names the stream it acts on; the router resolves
//! it strictly, so a missing stream never lands on the primary's branch.
//!
//! `vcs.*` go through the `Vcs` trait; `git.*` (rebase, cherry-pick,
//! revert, ignore) are git's own and call the git provider directly.
//! After a run the stream's workspace (and, when refs moved, its refs) is
//! announced, the way the watchers would.

use crate::commands::ops::Op;
use oxplow_domain::{Confirm, Invokers};
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use oxplow_domain::vcs::{ConflictChoice, OpOutcome, RemoteBranch, Vcs, VcsError};
use oxplow_domain::{CommandError, DomainError, StreamId};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{Handler, HandlerOutput, Invocation};
use crate::events::{EventBus, OxplowEvent, WorkspaceChangeKind};
use crate::vcs::GitProvider;
use crate::worktrees::WorktreeRouter;

/// What the commands run against.
#[derive(Clone)]
pub struct VcsTarget {
    pub vcs: Arc<dyn Vcs>,
    pub git: GitProvider,
    pub worktrees: Arc<WorktreeRouter>,
    pub events: EventBus,
    /// Where a run that moved refs says so.
    pub ref_moves: crate::ref_moves::RefMoves,
}

/// What a run changed, for the announcements that follow it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Touched {
    /// The stream's files only.
    Workspace,
    /// Its files and refs (a commit, a merge, a checkout).
    Refs,
    /// Refs every stream sees (fetched remotes, a renamed or deleted
    /// branch).
    AllRefs,
}

fn vcs_err(e: VcsError) -> CommandError {
    CommandError::from(DomainError::from(e))
}

/// Every input names its stream.
trait StreamInput {
    fn stream(&self) -> &str;
}

macro_rules! stream_input {
    ($($t:ty),*) => {
        $(impl StreamInput for $t {
            fn stream(&self) -> &str {
                &self.stream
            }
        })*
    };
}

/// One VCS operation of `capability`: parse the input, resolve the
/// stream's workspace strictly, run `op`, announce what it touched. A
/// person's only (agents run `git` in their terminal); `confirm` is the
/// least a command over it asks.
fn vcs_op<I, F, Fut>(
    capability: &str,
    name: &str,
    touched: Touched,
    confirm: Confirm,
    target: &VcsTarget,
    op: F,
) -> Op
where
    I: DeserializeOwned + JsonSchema + StreamInput + Send + 'static,
    F: Fn(VcsTarget, PathBuf, I) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Value, CommandError>> + Send + 'static,
{
    let target = target.clone();
    let op = Arc::new(op);
    Op::new(
        capability,
        name,
        serde_json::to_value(schemars::schema_for!(I)).expect("schema serializes"),
        false,
        Handler::External(Arc::new(move |_: Invocation, input: Value| {
            let target = target.clone();
            let op = op.clone();
            Box::pin(async move {
                let input: I =
                    serde_json::from_value(input).map_err(|e| CommandError::Invalid {
                        field: None,
                        message: e.to_string(),
                    })?;
                let stream: StreamId =
                    input.stream().parse().map_err(|_| CommandError::Invalid {
                        field: Some("/stream".into()),
                        message: format!("`{}` is not a stream id (`str1`)", input.stream()),
                    })?;
                let ws = target
                    .worktrees
                    .resolve_strict(Some(input.stream()))
                    .await?;
                let result = op(target.clone(), ws, input).await?;
                announce(&target, stream, touched).await;
                Ok(HandlerOutput {
                    result,
                    ..HandlerOutput::default()
                })
            })
        })),
    )
    .open_to(Invokers::HUMAN_ONLY)
    .confirm_at_least(confirm)
}

async fn announce(target: &VcsTarget, stream: StreamId, touched: Touched) {
    target.events.emit(OxplowEvent::WorkspaceChanged {
        stream_id: stream,
        change_kind: WorkspaceChangeKind::Updated,
        path: String::new(),
    });
    match touched {
        Touched::Workspace => {}
        Touched::Refs => target.ref_moves.moved(stream),
        Touched::AllRefs => {
            for (id, _) in target.worktrees.all().await.unwrap_or_default() {
                target.ref_moves.moved(id);
            }
        }
    }
}

fn outcome(o: OpOutcome) -> Result<Value, CommandError> {
    Ok(serde_json::to_value(o).expect("OpOutcome serializes"))
}

fn done() -> Result<Value, CommandError> {
    outcome(OpOutcome {
        success: true,
        ..OpOutcome::default()
    })
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommitInput {
    /// The stream whose workspace to commit (`str1`).
    pub stream: String,
    pub message: String,
    /// Commit untracked files too (default), not only changes to tracked
    /// ones.
    #[serde(default = "yes")]
    pub include_untracked: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PathsInput {
    pub stream: String,
    /// Workspace-relative paths.
    pub paths: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FetchInput {
    pub stream: String,
    /// The remote (default: the provider's, `origin`).
    #[serde(default)]
    pub remote: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoteBranchInput {
    pub stream: String,
    /// A remote branch; absent = the branch's own upstream.
    #[serde(default)]
    pub remote: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
}

impl RemoteBranchInput {
    fn remote_branch(&self) -> Result<Option<RemoteBranch>, CommandError> {
        match (&self.remote, &self.branch) {
            (Some(remote), Some(branch)) => Ok(Some(RemoteBranch {
                remote: remote.clone(),
                branch: branch.clone(),
            })),
            (None, None) => Ok(None),
            _ => Err(CommandError::Invalid {
                field: Some("/branch".into()),
                message: "name both `remote` and `branch`, or neither".into(),
            }),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RevInput {
    pub stream: String,
    /// A revision in the VCS's own terms: a branch, a sha, `HEAD`.
    pub rev: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckoutInput {
    pub stream: String,
    /// The branch to switch to.
    pub name: String,
    /// Create it at the current head first.
    #[serde(default)]
    pub create: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenameBranchInput {
    pub stream: String,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteBranchInput {
    pub stream: String,
    pub name: String,
    /// Delete even when not merged.
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResolveConflictInput {
    pub stream: String,
    pub path: String,
    /// `ours`, `theirs`, or `auto` (oxplow's smart merge, refused when the
    /// edits overlap).
    pub choice: ConflictChoice,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IgnoreInput {
    pub stream: String,
    /// A `.gitignore` pattern.
    pub entry: String,
}

stream_input!(
    CommitInput,
    PathsInput,
    FetchInput,
    RemoteBranchInput,
    RevInput,
    CheckoutInput,
    RenameBranchInput,
    DeleteBranchInput,
    ResolveConflictInput,
    IgnoreInput
);

pub fn ops(target: VcsTarget) -> Vec<Op> {
    let t = &target;
    vec![
        vcs_op(
            "vcs.write",
            "commit",
            Touched::Refs,
            Confirm::Never,
            t,
            |t, ws, i: CommitInput| async move {
                let rev = t
                    .vcs
                    .commit(
                        &ws,
                        oxplow_domain::vcs::CommitRequest {
                            message: i.message,
                            include_untracked: i.include_untracked,
                        },
                    )
                    .await
                    .map_err(vcs_err)?;
                Ok(json!({ "success": true, "revision": format!("{}:{rev}", t.vcs.rev_kind()) }))
            },
        ),
        vcs_op(
            "vcs.write",
            "stage",
            Touched::Workspace,
            Confirm::Never,
            t,
            |t, ws, i: PathsInput| async move {
                t.vcs.stage(&ws, &i.paths).await.map_err(vcs_err)?;
                done()
            },
        ),
        vcs_op(
            "vcs.write",
            "discard",
            Touched::Workspace,
            Confirm::Destructive,
            t,
            |t, ws, i: PathsInput| async move {
                t.vcs.discard(&ws, &i.paths).await.map_err(vcs_err)?;
                done()
            },
        ),
        vcs_op(
            "vcs.remote",
            "fetch",
            Touched::AllRefs,
            Confirm::Never,
            t,
            |t, ws, i: FetchInput| async move {
                outcome(
                    t.vcs
                        .fetch(&ws, i.remote.as_deref())
                        .await
                        .map_err(vcs_err)?,
                )
            },
        ),
        vcs_op(
            "vcs.remote",
            "pull",
            Touched::Refs,
            Confirm::Never,
            t,
            |t, ws, i: RemoteBranchInput| async move {
                let from = i.remote_branch()?;
                outcome(t.vcs.pull(&ws, from).await.map_err(vcs_err)?)
            },
        ),
        vcs_op(
            "vcs.remote",
            "push",
            Touched::AllRefs,
            Confirm::Never,
            t,
            |t, ws, i: RemoteBranchInput| async move {
                let to = i.remote_branch()?;
                outcome(t.vcs.push(&ws, to).await.map_err(vcs_err)?)
            },
        ),
        vcs_op(
            "vcs.write",
            "merge",
            Touched::Refs,
            Confirm::Destructive,
            t,
            |t, ws, i: RevInput| async move {
                outcome(t.vcs.merge(&ws, &i.rev).await.map_err(vcs_err)?)
            },
        ),
        vcs_op(
            "vcs.write",
            "checkout_branch",
            Touched::Refs,
            Confirm::Never,
            t,
            |t, ws, i: CheckoutInput| async move {
                t.vcs
                    .checkout_branch(&ws, &i.name, i.create)
                    .await
                    .map_err(vcs_err)?;
                done()
            },
        ),
        vcs_op(
            "vcs.write",
            "rename_branch",
            Touched::AllRefs,
            Confirm::Never,
            t,
            |t, ws, i: RenameBranchInput| async move {
                t.vcs
                    .rename_branch(&ws, &i.from, &i.to)
                    .await
                    .map_err(vcs_err)?;
                done()
            },
        ),
        vcs_op(
            "vcs.write",
            "delete_branch",
            Touched::AllRefs,
            Confirm::Destructive,
            t,
            |t, ws, i: DeleteBranchInput| async move {
                t.vcs
                    .delete_branch(&ws, &i.name, i.force)
                    .await
                    .map_err(vcs_err)?;
                done()
            },
        ),
        vcs_op(
            "vcs.write",
            "resolve_conflict",
            Touched::Workspace,
            Confirm::Never,
            t,
            |t, ws, i: ResolveConflictInput| async move {
                t.vcs
                    .resolve_conflict(&ws, &i.path, i.choice)
                    .await
                    .map_err(vcs_err)?;
                done()
            },
        ),
        vcs_op(
            "vcs.write",
            "rebase",
            Touched::Refs,
            Confirm::Destructive,
            t,
            |t, ws, i: RevInput| async move {
                outcome(t.git.rebase(&ws, &i.rev).await.map_err(vcs_err)?)
            },
        ),
        vcs_op(
            "vcs.write",
            "cherry_pick",
            Touched::Refs,
            Confirm::Never,
            t,
            |t, ws, i: RevInput| async move {
                outcome(t.git.cherry_pick(&ws, &i.rev).await.map_err(vcs_err)?)
            },
        ),
        vcs_op(
            "vcs.write",
            "revert",
            Touched::Refs,
            Confirm::Destructive,
            t,
            |t, ws, i: RevInput| async move {
                outcome(t.git.revert(&ws, &i.rev).await.map_err(vcs_err)?)
            },
        ),
        vcs_op(
            "vcs.write",
            "ignore",
            Touched::Workspace,
            Confirm::Never,
            t,
            |t, ws, i: IgnoreInput| async move {
                t.git.ignore(&ws, &i.entry).await.map_err(vcs_err)?;
                done()
            },
        ),
    ]
}

#[cfg(test)]
mod tests {
    use oxplow_domain::stores::StreamStore as _;
    use oxplow_domain::Actor;
    use serde_json::json;

    use crate::test_fixtures::{commit_all, services_with_effort};

    /// P5.B6 (tsk525): VCS mutations are a person's — an agent is denied;
    /// a destructive one needs the person's confirmation; the audit row
    /// holds what the VCS reported.
    #[tokio::test]
    async fn vcs_commands_are_a_persons_confirmed_and_audited() {
        let f = services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        commit_all(&root, "base");
        let stream = svc.stream_store.list().await.unwrap()[0].id.to_string();
        std::fs::write(root.join("a.txt"), "two\n").unwrap();

        let agent = Actor::Agent {
            thread_id: None,
            stream_id: None,
        };
        let err = svc
            .commands
            .run(
                &agent,
                "oxplow.vcs.commit",
                json!({ "stream": stream, "message": "by an agent" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(format!("{err:?}").contains("Denied"), "{err:?}");

        let discard = json!({ "stream": stream, "paths": ["a.txt"] });
        let err = svc
            .commands
            .run(&Actor::Human, "oxplow.vcs.discard", discard.clone(), false)
            .await
            .unwrap_err();
        assert!(format!("{err:?}").contains("NeedsConfirmation"), "{err:?}");
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "two\n"
        );

        let out = svc
            .commands
            .run(&Actor::Human, "oxplow.vcs.discard", discard, true)
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "one\n"
        );
        let audit = oxplow_db::SqliteCommandAuditStore::new(svc.db.clone())
            .get(out.audit_id.unwrap())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(audit.result.unwrap()["success"], json!(true));

        // A merge of the head is a no-op the VCS still reports on.
        let merged = svc
            .commands
            .run(
                &Actor::Human,
                "oxplow.vcs.merge",
                json!({ "stream": stream, "rev": "HEAD" }),
                true,
            )
            .await
            .unwrap();
        let audit = oxplow_db::SqliteCommandAuditStore::new(svc.db.clone())
            .get(merged.audit_id.unwrap())
            .await
            .unwrap()
            .unwrap();
        let result = audit.result.unwrap();
        assert_eq!(result["success"], json!(true), "{result}");
        assert_eq!(result["conflicts"], json!([]));

        // No stream, no run.
        let err = svc
            .commands
            .run(
                &Actor::Human,
                "oxplow.vcs.commit",
                json!({ "stream": "str999", "message": "m" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(format!("{err:?}").contains("str999"), "{err:?}");
    }
}
