//! Bringing an extension in from git (P8.A9): `oxplow.extension.install` and
//! `oxplow.extension.update`. `External` (a clone and a copy into a stream's
//! worktree) and `Confirm::Always`: what an extension can run is a
//! person's call, made against the review (`review_extension`) of the
//! commit they saw — `reviewed_sha` is the only commit installed. An
//! agent's run becomes a proposal a person approves. The files land as
//! ordinary project files in `oxplow/extensions/<name>/`, to commit for
//! the team.

use crate::commands::ops::Op;
use std::sync::Arc;

use oxplow_domain::{CommandError, Confirm, DomainError, StreamId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::util::{invalid, parse, ref_id, schema};
use super::{Handler, HandlerOutput, Invocation};
use crate::worktrees::WorktreeRouter;

pub const INSTALL: &str = "oxplow.extension.install";
pub const UPDATE: &str = "oxplow.extension.update";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstallInput {
    /// The repo whose root holds `extension.yaml`.
    pub git_url: String,
    /// A branch or tag; absent, the default branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_ref: Option<String>,
    /// The commit the person reviewed — the only one installed.
    pub reviewed_sha: String,
    /// The stream whose worktree takes it (`stream:str2`); absent, the
    /// primary checkout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateInput {
    /// The installed extension's name.
    pub name: String,
    /// The commit the person reviewed.
    pub reviewed_sha: String,
    /// As for `oxplow.extension.install`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<String>,
}

/// Where installs land.
#[derive(Clone)]
pub struct InstallDeps {
    pub worktrees: Arc<WorktreeRouter>,
}

/// The stream named; `None` is the primary checkout. Never the actor's:
/// these commands always ask, so the handler only runs as the person who
/// confirmed — and the approval shows the input as it will run (tsk786).
/// Where an install goes: the stream named (by ref), strictly — one that
/// isn't there is refused, never taken to mean the primary — else the
/// primary checkout.
async fn root_for(
    worktrees: &crate::worktrees::WorktreeRouter,
    named: Option<&str>,
) -> Result<std::path::PathBuf, CommandError> {
    match named {
        None => Ok(worktrees.resolve(None).await.into_local_path()),
        Some(raw) => {
            let stream: StreamId = ref_id(raw, "stream", "/stream")?;
            worktrees
                .resolve_strict(Some(&stream.to_string()))
                .await
                .map(|root| root.into_local_path())
                .map_err(|e| invalid("/stream", e.to_string()))
        }
    }
}

fn result(ext: &crate::extensions::Extension) -> HandlerOutput {
    HandlerOutput {
        result: serde_json::to_value(ext).expect("an extension serializes"),
        ..HandlerOutput::default()
    }
}

/// `extension.install { git_url, git_ref?, reviewed_sha, stream? }`.
pub fn install_op(deps: InstallDeps) -> Op {
    Op::new(
        "extensions.install",
        "install",
        schema::<InstallInput>(),
        false,
        Handler::External(Arc::new(move |_: Invocation, input| {
            let deps = deps.clone();
            Box::pin(async move {
                let input: InstallInput = parse(input)?;
                let root = root_for(&deps.worktrees, input.stream.as_deref()).await?;
                let project = deps.worktrees.project_dir().to_path_buf();
                let ext = tokio::task::spawn_blocking(move || {
                    crate::extensions::install_extension(
                        &root,
                        &project,
                        &input.git_url,
                        input.git_ref.as_deref(),
                        &input.reviewed_sha,
                    )
                })
                .await
                .map_err(|e| CommandError::Failed {
                    message: format!("install task panicked: {e}"),
                })??;
                Ok(result(&ext))
            })
        })),
    )
    .confirm_at_least(Confirm::Always)
}

/// `extension.update { name, reviewed_sha, stream? }`.
pub fn update_op(deps: InstallDeps) -> Op {
    Op::new(
        "extensions.install",
        "update",
        schema::<UpdateInput>(),
        false,
        Handler::External(Arc::new(move |_: Invocation, input| {
            let deps = deps.clone();
            Box::pin(async move {
                let input: UpdateInput = parse(input)?;
                let root = root_for(&deps.worktrees, input.stream.as_deref()).await?;
                let name = input.name.clone();
                let project = deps.worktrees.project_dir().to_path_buf();
                let ext = tokio::task::spawn_blocking(move || {
                    crate::extensions::update_extension(&root, &project, &name, &input.reviewed_sha)
                })
                .await
                .map_err(|e| CommandError::Failed {
                    message: format!("update task panicked: {e}"),
                })?
                .map_err(|e| match e {
                    DomainError::NotFound => CommandError::Invalid {
                        field: Some("/name".into()),
                        message: format!(
                            "no extension `{}` under oxplow/extensions/ in {}",
                            input.name,
                            input.stream.as_deref().unwrap_or("the primary checkout")
                        ),
                    },
                    other => other.into(),
                })?;
                Ok(result(&ext))
            })
        })),
    )
    .confirm_at_least(Confirm::Always)
}

pub fn ops(deps: InstallDeps) -> Vec<Op> {
    vec![install_op(deps.clone()), update_op(deps)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::Actor;

    /// tsk786: where an install lands is the input's `stream`, or the
    /// primary checkout — never the actor's: the handler only runs as the
    /// person who confirmed (an agent's run is a proposal), so "an agent's
    /// own stream" could never apply, and the approval shows the input as
    /// it will run.
    #[tokio::test]
    async fn an_install_lands_in_the_named_stream_or_the_primary_checkout() {
        let fx = services_with_effort().await;
        let primary = fx.svc.worktrees.resolve(None).await.into_local_path();
        let named = stream_ref(fx.svc.streams.list_streams().await.unwrap()[0].id);
        assert_eq!(
            root_for(&fx.svc.worktrees, Some(&named)).await.unwrap(),
            primary
        );
        assert_eq!(root_for(&fx.svc.worktrees, None).await.unwrap(), primary);
        assert!(root_for(&fx.svc.worktrees, Some("str1")).await.is_err());
    }
    use crate::test_fixtures::services_with_effort;
    use oxplow_domain::refs::build::stream_ref;
    use serde_json::json;

    /// An agent's install never runs: it becomes a proposal for a person.
    #[tokio::test]
    async fn an_agents_install_is_a_proposal() {
        let fx = services_with_effort().await;
        let err = fx
            .svc
            .commands
            .run(
                &Actor::Agent {
                    session_id: None,
                    thread_id: Some(fx.thread),
                    stream_id: None,
                },
                INSTALL,
                json!({ "git_url": "https://example.invalid/x.git", "reviewed_sha": "abc" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Proposed { .. }), "{err:?}");
        assert!(!fx._dir.path().join("oxplow/extensions").exists());
    }

    /// A named stream that isn't there — archived before a person approved
    /// the install — is refused, never taken to mean the primary checkout.
    #[tokio::test]
    async fn an_install_into_a_stream_that_isnt_there_is_refused() {
        let fx = services_with_effort().await;
        for name in [INSTALL, UPDATE] {
            let input = if name == INSTALL {
                json!({ "git_url": "https://example.invalid/x.git", "reviewed_sha": "abc", "stream": "stream:str99" })
            } else {
                json!({ "name": "x", "reviewed_sha": "abc", "stream": "stream:str99" })
            };
            let err = fx
                .svc
                .commands
                .run(&Actor::Human, name, input, true)
                .await
                .unwrap_err();
            assert!(
                matches!(&err, CommandError::Invalid { field: Some(f), message }
                    if f == "/stream" && message.contains("no stream")),
                "{name}: {err:?}"
            );
        }
        assert!(!fx._dir.path().join("oxplow/extensions").exists());
    }
}
