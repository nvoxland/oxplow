//! `extensions.deploy` operations: `oxplow.extension.deploy`, `rollback`,
//! `enable`, `disable`, `remove`, `share` and `unshare`
//! (`crate::extension::extension_deploy` does the work). External: each writes files
//! (the store, the settings, `.oxplow/.gitignore`) and activates, outside
//! the bus's transaction. A deploy returns once its deployment runs.

use std::sync::Arc;

use oxplow_domain::{CommandError, Confirm, Preview};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::agent_reads::actor_stream;
use super::ops::Op;
use super::util::{failed, parse, schema, ServicesSlot};
use super::{Handler, HandlerOutput, Invocation};
use crate::extension::extension_deploy::DeployError;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeployInput {
    /// The extension's name. Its source is the primary checkout's (another
    /// stream's copy is a draft, never deployed).
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WritePageInput {
    /// The extension whose source gets the page; created when missing.
    pub extension: String,
    /// The page's name: the file `pages/<page>.yaml`.
    pub page: String,
    /// The page's YAML text.
    pub spec: String,
    /// Deploy the extension afterwards.
    #[serde(default)]
    pub deploy: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewDashboardInput {
    /// What the dashboard is called; its page's name comes from it.
    pub title: String,
    /// The extension that holds it (default `my-pages`, created when
    /// missing).
    #[serde(default)]
    pub extension: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RollbackInput {
    /// The extension's name.
    pub name: String,
    /// The deployment to make active: its hash, or the start of it
    /// (`v_extension_deployment.hash`); absent, the one before the active one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NameInput {
    /// The extension's name.
    pub name: String,
}

fn domain(e: oxplow_domain::DomainError) -> CommandError {
    match e {
        oxplow_domain::DomainError::Invalid(message) => CommandError::Invalid {
            field: None,
            message,
        },
        other => failed(other.to_string()),
    }
}

/// What a person is asked: to turn an extension on, or to take a version of
/// one that adds to what it runs with (`details` lists only what it adds).
fn approval_summary(command: &str, name: &str, turns_on: bool) -> String {
    if turns_on {
        format!("`{name}` is turned off. Deploy it and turn it on, with:")
    } else if command.ends_with(".rollback") {
        format!("Roll `{name}` back? That version asks for:")
    } else if command.ends_with(".enable") {
        format!("Turn on `{name}`, with:")
    } else {
        format!("Deploy the new version of `{name}`? It asks for:")
    }
}

/// A deploy, rollback or enable that adds permissions asks a person
/// first: the bus asks them inline, or turns an agent's call into a
/// proposal; their confirmed call runs it.
fn deploy_error(command: &str, input: &serde_json::Value) -> impl Fn(DeployError) -> CommandError {
    let (command, input) = (command.to_string(), input.clone());
    move |e| match e {
        DeployError::NeedsApproval {
            name,
            adds,
            turns_on,
        } => CommandError::NeedsConfirmation {
            preview: Box::new(Preview {
                summary: approval_summary(&command, &name, turns_on),
                command: command.clone(),
                input: input.clone(),
                destructive: false,
                details: adds,
            }),
        },
        DeployError::Failed(e) => domain(e),
    }
}

fn output(value: impl Serialize) -> HandlerOutput {
    HandlerOutput {
        result: serde_json::to_value(value).expect("a result serializes"),
        ..HandlerOutput::default()
    }
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    vec![
        deploy_op(services.clone()),
        rollback_op(services.clone()),
        write_page_op(services.clone()),
        new_dashboard_op(services.clone()),
        enable_op(services.clone(), "enable", true),
        enable_op(services.clone(), "disable", false),
        remove_op(services.clone()),
        share_op(services.clone(), "share"),
        share_op(services, "unshare"),
    ]
}

fn deploy_op(services: ServicesSlot) -> Op {
    Op::new(
        "extensions.deploy",
        "deploy",
        schema::<DeployInput>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let raw = input.clone();
                let input: DeployInput = parse(input)?;
                let svc = services.get()?;
                let stream = actor_stream(&svc, invocation.actor())
                    .await?
                    .as_deref()
                    .and_then(oxplow_domain::StreamId::try_from_str);
                crate::extension::extension_deploy::refuse_draft_deploy(&svc, stream, &input.name)
                    .await
                    .map_err(domain)?;
                let deployed = crate::extension::extension_deploy::deploy(
                    &svc,
                    &input.name,
                    invocation.confirmed,
                )
                .await
                .map_err(deploy_error("oxplow.extension.deploy", &raw))?;
                Ok(output(deployed))
            })
        })),
    )
}

fn write_page_op(services: ServicesSlot) -> Op {
    Op::new(
        "pages.author",
        "write_page",
        schema::<WritePageInput>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let raw = input.clone();
                let input: WritePageInput = parse(input)?;
                let svc = services.get()?;
                let stream = actor_stream(&svc, invocation.actor())
                    .await?
                    .as_deref()
                    .and_then(oxplow_domain::StreamId::try_from_str);
                crate::extension::extension_deploy::refuse_draft_deploy(
                    &svc,
                    stream,
                    &input.extension,
                )
                .await
                .map_err(domain)?;
                let written = crate::extension::extension_deploy::write_page(
                    &svc,
                    &input.extension,
                    &input.page,
                    &input.spec,
                    input.deploy,
                    invocation.confirmed,
                )
                .await
                .map_err(deploy_error("oxplow.extension.write_page", &raw))?;
                Ok(output(written))
            })
        })),
    )
}

fn new_dashboard_op(services: ServicesSlot) -> Op {
    Op::new(
        "pages.author",
        "new_dashboard",
        schema::<NewDashboardInput>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let raw = input.clone();
                let input: NewDashboardInput = parse(input)?;
                let svc = services.get()?;
                let extension = input.extension.as_deref().unwrap_or("my-pages");
                let stream = actor_stream(&svc, invocation.actor())
                    .await?
                    .as_deref()
                    .and_then(oxplow_domain::StreamId::try_from_str);
                crate::extension::extension_deploy::refuse_draft_deploy(&svc, stream, extension)
                    .await
                    .map_err(domain)?;
                let written = crate::extension::extension_deploy::new_dashboard(
                    &svc,
                    extension,
                    &input.title,
                    invocation.confirmed,
                )
                .await
                .map_err(deploy_error("oxplow.extension.new_dashboard", &raw))?;
                Ok(output(written))
            })
        })),
    )
}

fn rollback_op(services: ServicesSlot) -> Op {
    Op::new(
        "extensions.deploy",
        "rollback",
        schema::<RollbackInput>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let raw = input.clone();
                let input: RollbackInput = parse(input)?;
                let svc = services.get()?;
                let done = crate::extension::extension_deploy::rollback(
                    &svc,
                    &input.name,
                    input.to.as_deref(),
                    invocation.confirmed,
                )
                .await
                .map_err(deploy_error("oxplow.extension.rollback", &raw))?;
                Ok(output(done))
            })
        })),
    )
}

fn enable_op(services: ServicesSlot, op: &'static str, enabled: bool) -> Op {
    Op::new(
        "extensions.deploy",
        op,
        schema::<NameInput>(),
        false,
        Handler::External(Arc::new(move |invocation: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let raw = input.clone();
                let input: NameInput = parse(input)?;
                let svc = services.get()?;
                crate::extension::extension_deploy::set_enabled(
                    &svc,
                    &input.name,
                    enabled,
                    invocation.confirmed,
                )
                .await
                .map_err(deploy_error(&format!("oxplow.extension.{op}"), &raw))?;
                Ok(output(
                    serde_json::json!({ "name": input.name, "enabled": enabled }),
                ))
            })
        })),
    )
}

fn remove_op(services: ServicesSlot) -> Op {
    Op::new(
        "extensions.deploy",
        "remove",
        schema::<NameInput>(),
        false,
        Handler::External(Arc::new(move |_: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let input: NameInput = parse(input)?;
                let svc = services.get()?;
                crate::extension::extension_deploy::remove(&svc, &input.name)
                    .await
                    .map_err(domain)?;
                Ok(output(serde_json::json!({ "name": input.name })))
            })
        })),
    )
    .confirm_at_least(Confirm::Destructive)
}

fn share_op(services: ServicesSlot, op: &'static str) -> Op {
    Op::new(
        "extensions.deploy",
        op,
        schema::<NameInput>(),
        false,
        Handler::External(Arc::new(move |_: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let input: NameInput = parse(input)?;
                let svc = services.get()?;
                let shared = op == "share";
                if shared {
                    crate::extension::extension_deploy::share(&svc, &input.name)
                } else {
                    crate::extension::extension_deploy::unshare(&svc, &input.name)
                }
                .map_err(domain)?;
                Ok(output(
                    serde_json::json!({ "name": input.name, "shared": shared }),
                ))
            })
        })),
    )
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use oxplow_domain::stores::StreamStore as _;
    use oxplow_domain::{Actor, CommandError, StreamId, StreamKind};
    use serde_json::json;

    use crate::Services;

    fn manifest(name: &str) -> String {
        format!("manifest: 2\nname: {name}\nintent:\n  purpose: test\n")
    }

    fn write(dir: &Path, rel: &str, text: &str) {
        let file = dir.join(rel);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }

    /// An agent on a new thread of `stream`.
    async fn agent_in(svc: &Services, stream: StreamId) -> Actor {
        let thread = crate::test_fixtures::new_thread(svc, stream, "t").await;
        Actor::Agent {
            session_id: None,
            thread_id: Some(thread.id),
            stream_id: Some(stream),
        }
    }

    /// A project with `acme` under its `.oxplow/extensions/`, and a second
    /// (worktree) stream whose checkout holds its own copy of it.
    async fn project_with_a_worktree() -> (
        tempfile::TempDir,
        tempfile::TempDir,
        std::sync::Arc<Services>,
        StreamId,
    ) {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        write(
            dir.path(),
            ".oxplow/extensions/acme/extension.yaml",
            &manifest("acme"),
        );
        let svc = Services::in_memory(dir.path()).await.unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        write(
            elsewhere.path(),
            ".oxplow/extensions/acme/extension.yaml",
            &manifest("acme"),
        );
        let mut other = svc.stream_store.list().await.unwrap().pop().unwrap();
        other.id = StreamId::placeholder();
        other.title = "other".into();
        other.branch = "feature".into();
        other.kind = StreamKind::Worktree;
        other.worktree_path = elsewhere.path().to_string_lossy().into_owned();
        let other_id = svc.stream_store.upsert(&other).await.unwrap();
        (dir, elsewhere, svc, other_id)
    }

    async fn deploy(
        svc: &Services,
        actor: &Actor,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, CommandError> {
        svc.commands
            .run(actor, "oxplow.extension.deploy", input, false)
            .await
            .map(|r| r.result)
    }

    /// An extension whose source is in the repo has one deployable copy:
    /// the main stream's. A worktree stream's copy is a draft, and its
    /// agent is told where to check it and when it deploys.
    #[tokio::test]
    async fn a_worktree_streams_agent_may_not_deploy_a_source_in_the_repo() {
        let (dir, _elsewhere, svc, other) = project_with_a_worktree().await;
        let err = deploy(
            &svc,
            &agent_in(&svc, other).await,
            json!({ "name": "acme" }),
        )
        .await
        .unwrap_err();
        let CommandError::Invalid { message, .. } = &err else {
            panic!("{err:?}");
        };
        assert!(
            message.contains("deploys only from the main stream"),
            "{message}"
        );
        assert!(message.contains("holds a draft"), "{message}");
        assert!(message.contains("oxplow_validate_extension"), "{message}");
        assert!(message.contains("`feature`"), "{message}");
        assert!(
            svc.extension_catalog.named(dir.path(), "acme").is_err(),
            "nothing was deployed"
        );
    }

    #[tokio::test]
    async fn the_primary_streams_agent_and_a_person_deploy_the_primary_checkouts_copy() {
        let (dir, _elsewhere, svc, _other) = project_with_a_worktree().await;
        let primary = svc.stream_store.primary().await.unwrap().unwrap().id;
        let out = deploy(
            &svc,
            &agent_in(&svc, primary).await,
            json!({ "name": "acme" }),
        )
        .await
        .unwrap();
        assert_eq!(out["already"], json!(false), "{out}");
        assert!(svc.extension_catalog.named(dir.path(), "acme").is_ok());
        let out = deploy(&svc, &Actor::Human, json!({ "name": "acme" }))
            .await
            .unwrap();
        assert_eq!(out["already"], json!(true), "{out}");
    }

    /// A source outside every checkout (an absolute `extensionSources`
    /// entry) has one copy, so any stream may deploy it.
    #[tokio::test]
    async fn a_source_outside_the_repo_deploys_from_any_stream() {
        let (dir, _elsewhere, svc, other) = project_with_a_worktree().await;
        let outside = tempfile::tempdir().unwrap();
        write(outside.path(), "extension.yaml", &manifest("mine"));
        write(
            dir.path(),
            ".oxplow/personal.yaml",
            &format!("extensionSources:\n  mine: {}\n", outside.path().display()),
        );
        let out = deploy(
            &svc,
            &agent_in(&svc, other).await,
            json!({ "name": "mine" }),
        )
        .await
        .unwrap();
        assert_eq!(out["name"], json!("mine"), "{out}");
    }

    /// The source is never the caller's to choose.
    #[tokio::test]
    async fn a_deploy_names_no_stream() {
        let (_dir, _elsewhere, svc, _other) = project_with_a_worktree().await;
        let err = deploy(
            &svc,
            &Actor::Human,
            json!({ "name": "acme", "stream": "str1" }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CommandError::Invalid { .. }), "{err:?}");
    }
}

#[cfg(test)]
mod approval_tests {
    use super::approval_summary;

    /// Turning an extension on and taking a new version are different
    /// questions, and each says which it is.
    #[test]
    fn the_question_says_which_it_is() {
        let new = approval_summary("oxplow.extension.deploy", "acme", false);
        assert!(new.contains("new version"), "{new}");
        let off = approval_summary("oxplow.extension.deploy", "acme", true);
        assert!(
            off.contains("turned off") && off.contains("turn it on"),
            "{off}"
        );
        let back = approval_summary("oxplow.extension.rollback", "acme", false);
        assert!(back.contains("Roll"), "{back}");
        let on = approval_summary("oxplow.extension.enable", "acme", false);
        assert!(on.contains("Turn on"), "{on}");
    }
}
