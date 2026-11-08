//! `vcs.*` and `git.*`: version-control mutations as commands (P5.B6,
//! `.context/vcs.md`, `.context/commands.md`). Each drives the VCS — a
//! system the bus doesn't own — so each is `External`: it runs, then the
//! bus audits it, the result (the VCS's `OpOutcome`, conflicts included)
//! on the audit row. A person's or an agent's — an agent's on its own
//! stream only, as its terminal's `git` reaches only its own worktree; a
//! destructive one asks a person first. Only `commit` is undoable (its inverse is `uncommit`, a soft reset of an unpushed commit). Each names the stream it acts on; the router resolves
//! it strictly, so a missing stream never lands on the primary's branch.
//!
//! `vcs.*` go through the `Vcs` trait; `git.*` (rebase, cherry-pick,
//! revert, ignore) are git's own and call the git provider directly.
//! After a run the stream's workspace (and, when refs moved, its refs) is
//! announced, the way the watchers would.

use crate::commands::ops::Op;
use oxplow_domain::Confirm;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use oxplow_domain::vcs::{ConflictChoice, OpOutcome, RemoteBranch, Vcs, VcsError};
use oxplow_domain::{CommandCall, CommandError, DomainError, StreamId};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::util::{bare_id, parse, schema};
use super::{Handler, HandlerOutput, Invocation};
use crate::events::{EventBus, OxplowEvent};
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
/// person's, an agent's (on its own stream only) or a page's for either.
/// `confirm` is the least a command over it asks.
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
    vcs_op_with_inverse(
        capability,
        name,
        touched,
        confirm,
        false,
        target,
        move |t, ws, i| {
            let ran = op(t, ws, i);
            async move { ran.await.map(|v| (v, None)) }
        },
    )
}

/// `vcs_op` for an operation that can say how it is undone: the op
/// answers its result and, when there is something to undo, the call that
/// does it.
fn vcs_op_with_inverse<I, F, Fut>(
    capability: &str,
    name: &str,
    touched: Touched,
    confirm: Confirm,
    undoable: bool,
    target: &VcsTarget,
    op: F,
) -> Op
where
    I: DeserializeOwned + JsonSchema + StreamInput + Send + 'static,
    F: Fn(VcsTarget, PathBuf, I) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<(Value, Option<CommandCall>), CommandError>> + Send + 'static,
{
    let target = target.clone();
    let op = Arc::new(op);
    Op::new(
        capability,
        name,
        schema::<I>(),
        undoable,
        Handler::External(Arc::new(move |invocation: Invocation, input: Value| {
            let target = target.clone();
            let op = op.clone();
            Box::pin(async move {
                let input: I = parse(input)?;
                let stream: StreamId = bare_id(input.stream(), oxplow_domain::uri::resources::STREAM, "/stream")?;
                // Isolation: an agent, or an effect in its event's stream,
                // works in its own stream's worktree only.
                if invocation.actor().confined_to_own_stream()
                    && invocation.actor().stream_id() != Some(stream)
                {
                    return Err(CommandError::Denied {
                        reason: format!(
                            "an agent or an effect runs VCS commands on its own stream only, not `{stream}`"
                        ),
                    });
                }
                let ws = target
                    .worktrees
                    .resolve_strict(Some(&stream.to_string()))
                    .await?
                    .into_local_path();
                let ran = op(target.clone(), ws, input).await;
                let announce = announcement(&target, stream, touched).await;
                match ran {
                    // Once the run is recorded: what shows the workspace
                    // and its refs reads them again.
                    Ok((result, inverse)) => Ok(HandlerOutput {
                        result,
                        inverse,
                        after_commit: Some(Box::new(announce)),
                        ..HandlerOutput::default()
                    }),
                    // A failed op may have touched the workspace too (a pull
                    // that stopped partway): say so now, as nothing commits.
                    Err(e) => {
                        announce();
                        Err(e)
                    }
                }
            })
        })),
    )
    .confirm_at_least(confirm)
}

/// What a run that touched `stream` announces: its workspace changed, and
/// — as `touched` says — its refs or every stream's moved. Made ready
/// here (finding every stream is async), sent by the caller.
async fn announcement(
    target: &VcsTarget,
    stream: StreamId,
    touched: Touched,
) -> impl FnOnce() + Send + Sync + 'static {
    let moved: Vec<StreamId> = match touched {
        Touched::Workspace => Vec::new(),
        Touched::Refs => vec![stream],
        Touched::AllRefs => target
            .worktrees
            .all()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|(id, _)| id)
            .collect(),
    };
    let (events, ref_moves) = (target.events.clone(), target.ref_moves.clone());
    move || {
        events.emit(OxplowEvent::WorkspaceChanged {
            stream_id: stream,
            paths: Vec::new(),
            overflow: false,
        });
        ref_moves.index_moved(stream);
        for id in moved {
            ref_moves.moved(id);
        }
    }
}

/// A VCS op's answer. One that failed outright — a rejected push, a
/// remote it couldn't reach — is the run's failure, git's log its message,
/// so its audit row says `error` rather than `ok` with a result that says
/// otherwise. One that stopped at conflicts did what it does — the
/// repository is mid-merge for a person to resolve — and answers with them,
/// readable on its row after the fact.
fn outcome(o: OpOutcome) -> Result<Value, CommandError> {
    if !o.success && o.conflicts.is_empty() {
        let log = o.log.trim();
        return Err(CommandError::Failed {
            message: if log.is_empty() {
                "git reported a failure and said nothing more".into()
            } else {
                log.to_string()
            },
        });
    }
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

/// The inverse of `commit`: take the commit back, its changes staying
/// staged.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UncommitInput {
    /// The stream whose workspace holds the commit (`str1`).
    pub stream: String,
    /// The commit to take back; it must still be the head.
    pub rev: String,
    /// Its parent, where the head returns to.
    pub parent: String,
}

/// Run by `commit`'s undo.
const UNCOMMIT: &str = "oxplow.vcs.uncommit";

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
    UncommitInput,
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
        vcs_op_with_inverse(
            "vcs.write",
            "commit",
            Touched::Refs,
            Confirm::Never,
            true,
            t,
            |t, ws, i: CommitInput| async move {
                // The head the commit builds on; a first commit has none,
                // and nothing to move back to.
                let parent = t.vcs.head(&ws).await.map_err(vcs_err)?.revision;
                let stream = i.stream.clone();
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
                let inverse = parent.map(|parent| CommandCall {
                    name: UNCOMMIT.into(),
                    input: json!({ "stream": stream, "rev": rev, "parent": parent }),
                });
                Ok((
                    json!({
                        "success": true,
                        "revision": format!("{}:{rev}", t.vcs.rev_kind()),
                        // The commit's id in its VCS (a git sha), for what takes it bare.
                        "rev": rev,
                    }),
                    inverse,
                ))
            },
        ),
        vcs_op(
            "vcs.write",
            "uncommit",
            Touched::Refs,
            Confirm::Never,
            t,
            |t, ws, i: UncommitInput| async move {
                t.vcs
                    .uncommit(&ws, &i.rev, &i.parent)
                    .await
                    .map_err(vcs_err)?;
                done()
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
            Confirm::Always,
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
            "vcs.remote",
            "publish",
            Touched::Refs,
            Confirm::Never,
            t,
            |t, ws, i: FetchInput| async move {
                outcome(
                    t.git
                        .publish(&ws, i.remote.as_deref())
                        .await
                        .map_err(vcs_err)?,
                )
            },
        ),
        vcs_op(
            "vcs.write",
            "rebase",
            Touched::Refs,
            Confirm::Always,
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

    /// A git op that failed outright — a push with nowhere to go — is the
    /// run's failure, its log the message, and its audit row says `error`:
    /// never an `ok` row whose result says it failed.
    #[tokio::test]
    async fn a_failed_git_op_is_a_failed_run() {
        let f = services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        commit_all(&root, "base");
        let stream = svc.stream_store.list().await.unwrap()[0].id.to_string();
        let err = svc
            .commands
            .run(
                &Actor::Human,
                "oxplow.vcs.push",
                json!({ "stream": stream, "remote": "nowhere", "branch": "main" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, oxplow_domain::CommandError::Failed { message } if message.contains("nowhere")),
            "{err:?}"
        );
        let audits = oxplow_db::SqliteCommandAuditStore::new(svc.db.clone())
            .list_recent(5)
            .await
            .unwrap();
        let push = audits
            .iter()
            .find(|a| a.command == "oxplow.vcs.push")
            .expect("the push is audited");
        assert_eq!(
            push.outcome,
            oxplow_domain::events::schema::CommandOutcome::Error
        );
    }

    /// An op that stopped at conflicts did what it does — the repository is
    /// mid-merge for a person — and answers with them; one that failed with
    /// nothing to say still says so.
    #[test]
    fn an_outcome_fails_unless_it_stopped_at_conflicts() {
        use oxplow_domain::vcs::OpOutcome;
        let conflicted = OpOutcome {
            success: false,
            log: "CONFLICT".into(),
            conflicts: vec!["a.txt".into()],
            auto_resolved: 0,
        };
        assert_eq!(
            super::outcome(conflicted).unwrap()["conflicts"],
            json!(["a.txt"])
        );
        let silent = super::outcome(OpOutcome::default())
            .unwrap_err()
            .to_string();
        assert!(silent.contains("git reported a failure"), "{silent}");
    }

    /// A page runs git for whoever pressed it: a person's destructive run
    /// asks them (the second click); an agent's doesn't — what an agent
    /// may call is its harness's to decide.
    #[tokio::test]
    async fn a_page_runs_git_for_its_presser_and_only_a_person_is_asked() {
        let f = services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        commit_all(&root, "base");
        let stream = svc.stream_store.list().await.unwrap()[0].id.to_string();
        let page = Actor::Page {
            page_id: "oxplow/uncommitted".into(),
            on_behalf_of: Box::new(Actor::Human),
        };
        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        svc.commands
            .run(
                &page,
                "oxplow.vcs.stage",
                json!({ "stream": stream, "paths": ["a.txt"] }),
                false,
            )
            .await
            .expect("a page stages for the person");
        svc.commands
            .run(
                &page,
                "oxplow.vcs.commit",
                json!({ "stream": stream, "message": "from a page" }),
                false,
            )
            .await
            .expect("a page commits for the person");
        std::fs::write(root.join("a.txt"), "three\n").unwrap();
        let discard = json!({ "stream": stream, "paths": ["a.txt"] });
        let err = svc
            .commands
            .run(&page, "oxplow.vcs.discard", discard.clone(), false)
            .await
            .unwrap_err();
        assert!(format!("{err:?}").contains("NeedsConfirmation"), "{err:?}");
        svc.commands
            .run(&page, "oxplow.vcs.discard", discard, true)
            .await
            .expect("confirmed, a page discards for the person");
        std::fs::write(root.join("a.txt"), "four\n").unwrap();
        let for_agent = Actor::Page {
            page_id: "oxplow/uncommitted".into(),
            on_behalf_of: Box::new(Actor::Agent {
                session_id: None,
                thread_id: Some(f.thread),
                stream_id: None,
            }),
        };
        svc.commands
            .run(
                &for_agent,
                "oxplow.vcs.discard",
                json!({ "stream": stream, "paths": ["a.txt"] }),
                false,
            )
            .await
            .expect("an agent's discard doesn't ask");
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "two\n"
        );
    }

    /// P5.B6 (tsk525): an agent runs VCS commands on its own stream only
    /// (as its terminal's git reaches only its own worktree); a destructive
    /// one needs a person's confirmation; the audit row holds what the VCS
    /// reported.
    #[tokio::test]
    async fn vcs_commands_are_an_agents_on_its_own_stream_confirmed_and_audited() {
        let f = services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        commit_all(&root, "base");
        let own = svc.stream_store.list().await.unwrap()[0].id;
        let stream = own.to_string();

        let agent = |stream_id| Actor::Agent {
            session_id: None,
            thread_id: Some(f.thread),
            stream_id,
        };
        std::fs::write(root.join("a.txt"), "by an agent\n").unwrap();
        svc.commands
            .run(
                &agent(Some(own)),
                "oxplow.vcs.commit",
                json!({ "stream": stream, "message": "by an agent" }),
                false,
            )
            .await
            .expect("an agent commits on its own stream");
        let elsewhere = oxplow_domain::StreamId::new(own.value() + 1);
        let err = svc
            .commands
            .run(
                &agent(Some(elsewhere)),
                "oxplow.vcs.commit",
                json!({ "stream": stream, "message": "not its stream" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(format!("{err:?}").contains("Denied"), "{err:?}");
        // An effect is held to its event's stream the same way: none, or
        // another, is refused.
        for stream_id in [None, Some(elsewhere)] {
            let effect = Actor::Effect {
                effect: "acme/ship".into(),
                thread_id: Some(f.thread),
                stream_id,
            };
            let err = svc
                .commands
                .run(
                    &effect,
                    "oxplow.vcs.commit",
                    json!({ "stream": stream, "message": "not its stream" }),
                    false,
                )
                .await
                .unwrap_err();
            assert!(format!("{err:?}").contains("Denied"), "{err:?}");
        }
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        commit_all(&root, "base again");
        std::fs::write(root.join("a.txt"), "two\n").unwrap();

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

    fn git(root: &std::path::Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A commit says how it is undone; undoing it moves the head back to
    /// its parent and leaves the changes staged, the files as they were.
    /// A first commit (no parent) has nothing to go back to.
    #[tokio::test]
    async fn a_commit_is_undone_by_a_soft_reset() {
        let f = services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        let base = commit_all(&root, "base");
        let stream = svc.stream_store.list().await.unwrap()[0].id.to_string();
        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        std::fs::write(root.join("new.txt"), "new\n").unwrap();
        let out = svc
            .commands
            .run(
                &Actor::Human,
                "oxplow.vcs.commit",
                json!({ "stream": stream, "message": "work" }),
                false,
            )
            .await
            .unwrap();
        let rev = out.result["rev"].as_str().unwrap().to_string();
        let inverse = out.inverse.clone().expect("a commit is undoable");
        assert_eq!(inverse.name, "oxplow.vcs.uncommit");
        assert_eq!(
            inverse.input,
            json!({ "stream": stream, "rev": rev, "parent": base })
        );
        svc.commands
            .undo(&Actor::Human, out.audit_id.unwrap(), false)
            .await
            .expect("the commit is undone");
        assert_eq!(git(&root, &["rev-parse", "HEAD"]), base);
        assert_eq!(
            git(&root, &["status", "--porcelain"]),
            "M  a.txt\nA  new.txt"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("a.txt")).unwrap(),
            "two\n"
        );
    }

    /// A commit made from a page's form (the Uncommitted page's, as the
    /// page acting for the person) comes back undoable too, and the person
    /// can undo it.
    #[tokio::test]
    async fn a_commit_a_page_makes_for_the_person_is_undoable() {
        let f = services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        let base = commit_all(&root, "base");
        let stream = svc.stream_store.list().await.unwrap()[0].id.to_string();
        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        let page = Actor::Page {
            page_id: "oxplow/uncommitted-changes#commit".into(),
            on_behalf_of: Box::new(Actor::Human),
        };
        let out = svc
            .commands
            .run(
                &page,
                "oxplow.vcs.commit",
                json!({ "stream": stream, "message": "work" }),
                false,
            )
            .await
            .unwrap();
        assert!(out.inverse.is_some(), "a page's commit is undoable");
        svc.commands
            .undo(&Actor::Human, out.audit_id.unwrap(), true)
            .await
            .expect("the person undoes it");
        assert_eq!(git(&root, &["rev-parse", "HEAD"]), base);
    }

    #[tokio::test]
    async fn a_first_commit_has_no_inverse() {
        let f = services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        let stream = svc.stream_store.list().await.unwrap()[0].id.to_string();
        // A repository with no commit yet.
        git(&root, &["update-ref", "-d", "HEAD"]);
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        let out = svc
            .commands
            .run(
                &Actor::Human,
                "oxplow.vcs.commit",
                json!({ "stream": stream, "message": "first" }),
                false,
            )
            .await
            .unwrap();
        assert!(out.inverse.is_none());
    }

    /// Undo is refused, and changes nothing, once something is committed
    /// on top, once a remote holds the commit, when the tree has other
    /// changes, or mid-merge.
    #[tokio::test]
    async fn undoing_a_commit_is_refused_when_it_moved_on_was_pushed_or_is_mid_merge() {
        let f = services_with_effort().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        commit_all(&root, "base");
        let stream = svc.stream_store.list().await.unwrap()[0].id.to_string();
        let commit = |body: &'static str| {
            let (svc, root, stream) = (svc.clone(), root.clone(), stream.clone());
            async move {
                std::fs::write(root.join("a.txt"), body).unwrap();
                svc.commands
                    .run(
                        &Actor::Human,
                        "oxplow.vcs.commit",
                        json!({ "stream": stream, "message": body }),
                        false,
                    )
                    .await
                    .unwrap()
            }
        };
        let refused = |out: crate::commands::CommandOutcome, why: &'static str| {
            let (svc, root) = (svc.clone(), root.clone());
            async move {
                let head = git(&root, &["rev-parse", "HEAD"]);
                let err = svc
                    .commands
                    .undo(&Actor::Human, out.audit_id.unwrap(), false)
                    .await
                    .unwrap_err();
                assert!(format!("{err:?}").contains(why), "{err:?}");
                assert_eq!(git(&root, &["rev-parse", "HEAD"]), head, "nothing moved");
            }
        };
        // Something on top.
        let first = commit("two\n").await;
        let _second = commit("three\n").await;
        refused(first, "no longer that commit").await;
        // Pushed.
        let third = commit("four\n").await;
        let sha = git(&root, &["rev-parse", "HEAD"]);
        git(&root, &["update-ref", "refs/remotes/origin/main", &sha]);
        refused(third, "was pushed").await;
        git(&root, &["update-ref", "-d", "refs/remotes/origin/main"]);
        // Changes in the tree since: a soft reset would mix them with the
        // commit's own in the index.
        let dirty = commit("four and a half\n").await;
        std::fs::write(root.join("other.txt"), "mine\n").unwrap();
        refused(dirty, "has changes").await;
        std::fs::remove_file(root.join("other.txt")).unwrap();
        // Mid-merge.
        let fourth = commit("five\n").await;
        let head = git(&root, &["rev-parse", "HEAD"]);
        std::fs::write(root.join(".git/MERGE_HEAD"), format!("{head}\n")).unwrap();
        refused(fourth, "in progress").await;
    }
}
