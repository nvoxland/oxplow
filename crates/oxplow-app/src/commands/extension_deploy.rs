//! `extensions.deploy` operations: `oxplow.extension.deploy`, `rollback`,
//! `enable`, `disable`, `remove`, `share` and `unshare`
//! (`crate::extension_deploy` does the work). External: each writes files
//! (the store, the settings, `.oxplow/.gitignore`) and activates, outside
//! the bus's transaction. A deploy returns once its deployment runs.

use std::sync::Arc;

use oxplow_domain::{CommandError, Confirm, Invokers, Preview};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::agent_reads::stream_or_actors;
use super::ops::Op;
use super::util::{failed, parse, schema, ServicesSlot};
use super::{Handler, HandlerOutput, Invocation};
use crate::extension_deploy::DeployError;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeployInput {
    /// The extension's name.
    pub name: String,
    /// The stream whose checkout holds the source (`str2`); absent, the
    /// caller's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<String>,
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

/// A deploy, rollback or enable that adds permissions asks a person
/// first: the bus asks them inline, or turns an agent's call into a
/// proposal; their confirmed call runs it.
fn deploy_error(command: &str, input: &serde_json::Value) -> impl Fn(DeployError) -> CommandError {
    let (command, input) = (command.to_string(), input.clone());
    move |e| match e {
        DeployError::NeedsApproval { name, adds } => CommandError::NeedsConfirmation {
            preview: Box::new(Preview {
                command: command.clone(),
                summary: format!("`{name}` asks for more than it runs with now"),
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
                let stream = stream_or_actors(&svc, invocation.actor(), input.stream).await?;
                let root = match stream.as_deref() {
                    None => svc.worktrees.resolve(None).await,
                    Some(stream) => svc
                        .worktrees
                        .resolve_strict(Some(stream))
                        .await
                        .map_err(|e| failed(e.to_string()))?,
                }
                .into_local_path();
                let from = stream
                    .as_deref()
                    .and_then(oxplow_domain::StreamId::try_from_str)
                    .map(|s| s.value());
                let deployed = crate::extension_deploy::deploy(
                    &svc,
                    &input.name,
                    &root,
                    from,
                    invocation.confirmed,
                )
                .await
                .map_err(deploy_error("oxplow.extension.deploy", &raw))?;
                Ok(output(deployed))
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
                let done = crate::extension_deploy::rollback(
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
                crate::extension_deploy::set_enabled(
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
    .open_to(Invokers::NO_AGENT)
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
                crate::extension_deploy::remove(&svc, &input.name)
                    .await
                    .map_err(domain)?;
                Ok(output(serde_json::json!({ "name": input.name })))
            })
        })),
    )
    .open_to(Invokers::NO_AGENT)
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
                    crate::extension_deploy::share(&svc, &input.name)
                } else {
                    crate::extension_deploy::unshare(&svc, &input.name)
                }
                .map_err(domain)?;
                Ok(output(
                    serde_json::json!({ "name": input.name, "shared": shared }),
                ))
            })
        })),
    )
    .open_to(Invokers::NO_AGENT)
}
