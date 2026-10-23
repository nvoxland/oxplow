//! Settings → Integrations as commands: an extension provider's instances
//! on this machine (`oxplow.provider.*`) and signing in for a credential
//! (`oxplow.provider.begin_sign_in` / `complete_sign_in` /
//! `cancel_sign_in`). `External`: starting, stopping and checking an
//! instance runs its program, and its credentials are in the OS keychain,
//! both outside oxplow's database. What an instance is, with its health, is
//! read as `v_provider_instance`. The credential a person types
//! (`oxplow.provider.set_credential`) is a secret like the collector's:
//! `writeOnly`, kept out of every record.

use std::sync::Arc;

use oxplow_domain::CommandError;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use super::ops::Op;
use super::util::{failed, invalid, parse, schema, ServicesSlot};
use super::{Handler, HandlerOutput, Invocation};
use crate::providers::Scope;

#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum InstanceScope {
    /// The project's: shared with the team.
    Project,
    /// The person's own on this machine, running in every project that has
    /// the extension.
    Global,
}

impl From<InstanceScope> for Scope {
    fn from(scope: InstanceScope) -> Self {
        match scope {
            InstanceScope::Project => Scope::Project,
            InstanceScope::Global => Scope::Global,
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckInstanceInput {
    /// `<extension>/<instance id>`.
    pub instance: String,
    /// The config to check it against; left out, an empty one.
    #[serde(default)]
    pub config: Option<Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetInstanceInput {
    /// `<extension>/<instance id>`.
    pub instance: String,
    /// Turned on (checked first: an unapproved or unconfigured instance is
    /// refused and nothing is written) or off.
    pub enabled: bool,
    /// Its config, as its provider declares it.
    #[serde(default)]
    pub config: Option<Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddInstanceInput {
    /// `<extension>/<instance id>`: the extension's provider is the
    /// instance id's, unless `provider` names another.
    pub instance: String,
    /// The provider of the extension this is another instance of.
    pub provider: String,
    pub scope: InstanceScope,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstanceInput {
    /// `<extension>/<instance id>`.
    pub instance: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetInstanceCredentialInput {
    /// `<extension>/<instance id>`.
    pub instance: String,
    /// The credential's name, as the provider declares it.
    pub name: String,
    /// Its value, kept in the OS keychain and never read back. Left out or
    /// blank, the credential is forgotten. A secret (`writeOnly`).
    #[serde(default)]
    #[schemars(extend("writeOnly" = true))]
    pub value: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BeginSignInInput {
    /// `<extension>/<instance id>`.
    pub instance: String,
    /// The credential, one its provider declares with `oauth:`.
    pub name: String,
    /// Where the redirect comes back to on the person's machine.
    pub redirect_port: u16,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CancelSignInInput {
    pub instance: String,
    pub name: String,
    /// The number `begin_sign_in` answered.
    pub sign_in: u32,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompleteSignInInput {
    pub instance: String,
    pub name: String,
    /// The path and query the browser asked for.
    pub redirect: String,
}

fn result<T: serde::Serialize>(value: &T) -> Result<HandlerOutput, CommandError> {
    Ok(HandlerOutput {
        result: serde_json::to_value(value).map_err(failed)?,
        ..HandlerOutput::default()
    })
}

fn domain(field: &str, e: oxplow_domain::DomainError) -> CommandError {
    match e {
        oxplow_domain::DomainError::Invalid(m) => invalid(field, m),
        other => failed(other),
    }
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    let ext = |scope: &str, name: &str, schema: Value, handler: Handler| {
        Op::new(scope, name, schema, false, handler)
    };
    let check = {
        let services = services.clone();
        ext(
            "providers.write",
            "check_instance",
            schema::<CheckInstanceInput>(),
            Handler::External(Arc::new(move |_: Invocation, input| {
                let services = services.clone();
                Box::pin(async move {
                    let input: CheckInstanceInput = parse(input)?;
                    let svc = services.get()?;
                    let view = svc
                        .providers
                        .check_instance(&input.instance, input.config.unwrap_or_else(|| json!({})))
                        .await?;
                    result(&view)
                })
            })),
        )
    };
    let set = {
        let services = services.clone();
        ext(
            "providers.write",
            "set_instance",
            schema::<SetInstanceInput>(),
            Handler::External(Arc::new(move |inv: Invocation, input| {
                let services = services.clone();
                Box::pin(async move {
                    let input: SetInstanceInput = parse(input)?;
                    let svc = services.get()?;
                    let view = svc
                        .providers
                        .set_instance(
                            inv.actor(),
                            &input.instance,
                            input.enabled,
                            input.config.unwrap_or_else(|| json!({})),
                        )
                        .await?;
                    result(&view)
                })
            })),
        )
    };
    let add = {
        let services = services.clone();
        ext(
            "providers.write",
            "add_instance",
            schema::<AddInstanceInput>(),
            Handler::External(Arc::new(move |inv: Invocation, input| {
                let services = services.clone();
                Box::pin(async move {
                    let input: AddInstanceInput = parse(input)?;
                    let svc = services.get()?;
                    svc.providers
                        .add_instance(
                            inv.actor(),
                            &input.instance,
                            &input.provider,
                            input.scope.into(),
                        )
                        .await?;
                    result(&json!({ "instance": input.instance }))
                })
            })),
        )
    };
    let off_here = {
        let services = services.clone();
        ext(
            "providers.write",
            "turn_off_here",
            schema::<InstanceInput>(),
            Handler::External(Arc::new(move |inv: Invocation, input| {
                let services = services.clone();
                Box::pin(async move {
                    let input: InstanceInput = parse(input)?;
                    let svc = services.get()?;
                    svc.providers.off_here(inv.actor(), &input.instance).await?;
                    result(&json!({ "instance": input.instance }))
                })
            })),
        )
    };
    let remove = {
        let services = services.clone();
        ext(
            "providers.write",
            "remove_instance",
            schema::<InstanceInput>(),
            Handler::External(Arc::new(move |inv: Invocation, input| {
                let services = services.clone();
                Box::pin(async move {
                    let input: InstanceInput = parse(input)?;
                    let svc = services.get()?;
                    svc.providers
                        .remove_instance(inv.actor(), &input.instance)
                        .await?;
                    result(&json!({ "instance": input.instance }))
                })
            })),
        )
    };
    let set_credential = {
        let services = services.clone();
        ext(
            "credentials.write",
            "set_instance_credential",
            schema::<SetInstanceCredentialInput>(),
            Handler::External(Arc::new(move |_: Invocation, input| {
                let services = services.clone();
                Box::pin(async move {
                    let input: SetInstanceCredentialInput = parse(input)?;
                    let svc = services.get()?;
                    let given = input.value.as_deref().is_some_and(|v| !v.trim().is_empty());
                    svc.providers
                        .set_credential(&input.instance, &input.name, input.value.as_deref())
                        .map_err(|e| domain("/name", e))?;
                    svc.providers.credential_changed(&input.instance).await;
                    result(&json!({ "instance": input.instance, "name": input.name, "set": given }))
                })
            })),
        )
    };
    let begin = {
        let services = services.clone();
        ext(
            "credentials.write",
            "begin_sign_in",
            schema::<BeginSignInInput>(),
            Handler::External(Arc::new(move |_: Invocation, input| {
                let services = services.clone();
                Box::pin(async move {
                    let input: BeginSignInInput = parse(input)?;
                    let svc = services.get()?;
                    let begun = svc
                        .providers
                        .begin_sign_in(&input.instance, &input.name, input.redirect_port)
                        .await
                        .map_err(|e| domain("/name", e))?;
                    result(&begun)
                })
            })),
        )
    };
    let cancel = {
        let services = services.clone();
        ext(
            "credentials.write",
            "cancel_sign_in",
            schema::<CancelSignInInput>(),
            Handler::External(Arc::new(move |_: Invocation, input| {
                let services = services.clone();
                Box::pin(async move {
                    let input: CancelSignInInput = parse(input)?;
                    let svc = services.get()?;
                    svc.providers
                        .cancel_sign_in(&input.instance, &input.name, input.sign_in)
                        .await;
                    result(&json!({ "instance": input.instance, "name": input.name }))
                })
            })),
        )
    };
    let complete = ext(
        "credentials.write",
        "complete_sign_in",
        schema::<CompleteSignInInput>(),
        Handler::External(Arc::new(move |_: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let input: CompleteSignInInput = parse(input)?;
                let svc = services.get()?;
                let done = svc
                    .providers
                    .complete_sign_in(&input.instance, &input.name, &input.redirect)
                    .await
                    .map_err(|e| domain("/redirect", e))?;
                result(&done)
            })
        })),
    );
    vec![
        check,
        set,
        add,
        off_here,
        remove,
        set_credential,
        begin,
        cancel,
        complete,
    ]
}
