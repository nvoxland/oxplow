//! Uniform public operations for every registered setting.

use std::sync::Arc;

use oxplow_config::settings::{Level, SettingsError};
use oxplow_domain::events::schema::{ConfigChanged, ConfigChangedV2, ConfigLayer};
use oxplow_domain::{CommandCall, CommandError, Envelope};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

use super::config_commands::ConfigTarget;
use super::{
    ops::Op,
    util::{parse, schema},
    Handler, HandlerOutput, Invocation,
};

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadInput {
    key: String,
    /// Omit to read the effective inherited value.
    level: Option<Level>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetInput {
    key: String,
    value: Value,
    level: Level,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UnsetInput {
    key: String,
    level: Level,
}

fn error(error: SettingsError) -> CommandError {
    CommandError::Invalid {
        field: Some("/key".into()),
        message: error.to_string(),
    }
}

/// Native config operations retain the live script owner's namespace when
/// composed. A command's broad config scope cannot widen the composing
/// extension's access to another extension's secrets or private preferences.
fn authorize_composer(
    target: &ConfigTarget,
    run: &oxplow_domain::run::RunContext,
    definition: &oxplow_config::settings::SettingDefinition,
    writing: bool,
) -> Result<(), CommandError> {
    if run.composed_by().is_none()
        || !(if writing {
            definition.owner.starts_with("extension/")
        } else {
            definition.secret
        })
    {
        return Ok(());
    }
    let denied = || CommandError::Denied {
        reason: "the composing extension does not own this setting".into(),
    };
    let catalog = target.catalog.as_ref().ok_or_else(denied)?;
    let host = crate::extension::extension_state::StateHost::new(
        catalog.clone(),
        target.project_dir.clone(),
    );
    let namespace = host
        .with_composer(run, |extension| extension.namespace.clone())
        .map_err(|_| denied())?;
    if definition.owner != format!("extension/{namespace}") {
        return Err(denied());
    }
    Ok(())
}

pub(super) fn ops(target: ConfigTarget) -> Vec<Op> {
    let list_target = target.clone();
    let list = Op::new(
        "config.read",
        "list_keys",
        schema::<super::config_commands::NoInput>(),
        false,
        Handler::Tx(Arc::new(move |_, input| {
            parse::<super::config_commands::NoInput>(input)?;
            let settings = list_target.settings()?;
            let definitions: Vec<_> = settings
                .registry
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .definitions()
                .cloned()
                .collect();
            Ok(HandlerOutput {
                result: json!(definitions),
                ..Default::default()
            })
        })),
    );
    let read_target = target.clone();
    let get = Op::new(
        "config.read",
        "get",
        schema::<ReadInput>(),
        false,
        Handler::Tx(Arc::new(move |ctx, input| {
            let input: ReadInput = parse(input)?;
            let settings = read_target.settings()?;
            let definition = settings.definition(&input.key).map_err(error)?;
            authorize_composer(&read_target, ctx.run, &definition, false)?;
            let value = settings.get(&input.key, input.level).map_err(error)?;
            authorize_composer(&read_target, ctx.run, &definition, false)?;
            ctx.run.used_scope("config.read");
            ctx.trace.count("config.read");
            if value.secret {
                ctx.trace.count("config.secret");
                ctx.run.trace().count("config.secret");
            }
            Ok(HandlerOutput {
                result: serde_json::to_value(value).expect("setting serializes"),
                ..Default::default()
            })
        })),
    );
    let set_target = target.clone();
    let set = Op::new(
        "config.write",
        "set",
        schema::<SetInput>(),
        true,
        Handler::External(Arc::new(move |invocation, input| {
            let target = set_target.clone();
            Box::pin(async move {
                let input: SetInput = parse(input)?;
                write(
                    target,
                    invocation,
                    input.key,
                    Some(input.value),
                    input.level,
                )
            })
        })),
    )
    .with_unrecorded(&["value"]);
    let unset = Op::new(
        "config.write",
        "unset",
        schema::<UnsetInput>(),
        true,
        Handler::External(Arc::new(move |invocation, input| {
            let target = target.clone();
            Box::pin(async move {
                let input: UnsetInput = parse(input)?;
                write(target, invocation, input.key, None, input.level)
            })
        })),
    );
    vec![list, get, set, unset]
}

fn write(
    target: ConfigTarget,
    invocation: Invocation,
    key: String,
    value: Option<Value>,
    level: Level,
) -> Result<HandlerOutput, CommandError> {
    let settings = target.settings()?;
    let definition = settings.definition(&key).map_err(error)?;
    authorize_composer(&target, &invocation.run, &definition, true)?;
    // A replacement or removal must not need to decrypt the old value (which
    // may have an obsolete endpoint binding). Secret writes have no inverse.
    let before = if definition.secret {
        None
    } else {
        settings.layer(level).map_err(error)?.remove(&key)
    };
    // A schema change can leave an existing override invalid. It must still
    // be replaceable or removable, but undo must not restore invalid data.
    let inverse = if definition.secret
        || before
            .as_ref()
            .is_some_and(|value| settings.validate(&key, value).is_err())
    {
        None
    } else {
        Some(match before.as_ref() {
            Some(value) => CommandCall {
                name: "oxplow.config.set".into(),
                input: json!({"key":key,"value":value,"level":level}),
            },
            None => CommandCall {
                name: "oxplow.config.unset".into(),
                input: json!({"key":key,"level":level}),
            },
        })
    };
    super::config_commands::preflight(&target, &[(key.clone(), value.clone())], level)?;
    match &value {
        Some(value) => settings.set(&key, value.clone(), level),
        None => settings.unset(&key, level),
    }
    .map_err(error)?;
    target.refresh()?;
    let event = Envelope::typed::<ConfigChanged>(
        invocation.run.actor().source(),
        &ConfigChangedV2 {
            key: key.clone(),
            before: before.unwrap_or(Value::Null),
            after: if definition.secret {
                Value::Null
            } else {
                value.unwrap_or(Value::Null)
            },
            layer: match level {
                Level::Local => ConfigLayer::Local,
                Level::Project => ConfigLayer::Project,
                Level::Global => ConfigLayer::Global,
            },
        },
    )
    .with_subject([oxplow_domain::uri::build::config_uri(&key)]);
    Ok(HandlerOutput {
        result: json!({"key":key,"level":level,"secret":definition.secret,"changed":true}),
        inverse,
        events: vec![event],
        ..Default::default()
    })
}

#[cfg(test)]
mod ownership_tests {
    use super::*;
    use crate::commands::TxCtx;
    use crate::extension::extension_private_state_tests::{declare, reconcile};
    use crate::test_fixtures::{services_with_thread, ThreadFixture};
    use oxplow_domain::run::{ContributionRef, Mode, RunContext};
    use oxplow_domain::Actor;

    const CONFIGURATION: &str = "configuration:
  version: 1
  schema:
    type: object
    properties:
      limit: { type: integer }
      token: { type: string, x-secret: true }
  defaults:
    limit: 1
";

    fn target(fx: &ThreadFixture) -> ConfigTarget {
        ConfigTarget {
            config: fx.svc.config.clone(),
            project_dir: fx.svc.layout.project_dir.clone(),
            global_dir: fx.svc.layout.state_dir.join("global-config"),
            secrets: fx.svc.secrets.clone(),
            catalog: Some(fx.svc.extension_catalog.clone()),
            events: fx.svc.events.clone(),
            applied: fx.svc.config_applied.clone(),
        }
    }

    fn composed_run(fx: &ThreadFixture) -> RunContext {
        let snapshot = fx
            .svc
            .extension_catalog
            .live_snapshot(&fx.svc.layout.project_dir);
        let extension = snapshot
            .extensions()
            .iter()
            .find(|extension| extension.name == "alpha")
            .unwrap();
        let owner = ContributionRef {
            id: extension
                .contributions
                .iter()
                .find(|item| item.id.kind == oxplow_domain::contribution::ContributionKind::Command)
                .unwrap()
                .id
                .clone(),
            extension: extension.name.clone(),
            version: extension.version.clone(),
            generation: Some(snapshot.generation().into()),
            deployment: extension.deployment.hash().map(str::to_owned),
        };
        let parent = RunContext::contribution(owner.clone(), Actor::Human, Mode::External);
        // Crossing into a native command retains the captured script owner.
        parent.child(Some(owner), Mode::External)
    }

    async fn native_get(fx: &ThreadFixture, run: RunContext, key: &str) -> Result<Value, String> {
        let Handler::Tx(handler) = ops(target(fx))
            .into_iter()
            .find(|op| op.name == "get")
            .unwrap()
            .handler
        else {
            unreachable!()
        };
        let key = key.to_owned();
        let vocabulary = fx.svc.commands.vocabulary().clone();
        fx.svc
            .db
            .read_or(move |conn| {
                let trace = oxplow_domain::scope::ScopeTrace::default();
                let ctx = TxCtx {
                    conn,
                    run: &run,
                    events: oxplow_db::EventCtx {
                        vocabulary: &vocabulary.current(),
                        source: run.actor().source(),
                        cause: None,
                    },
                    confirmed: true,
                    may_write: None,
                    depth: 1,
                    trace: &trace,
                };
                handler(&ctx, json!({"key":key}))
                    .map(|out| out.result)
                    .map_err(oxplow_db::TxError::Aborted)
            })
            .await
            .map_err(|error| format!("{error:?}"))
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn composed_native_settings_operations_cannot_cross_extension_ownership() {
        let fx = services_with_thread().await;
        declare(fx._dir.path(), "alpha", CONFIGURATION);
        declare(fx._dir.path(), "beta", CONFIGURATION);
        reconcile(&fx).await;
        let settings = target(&fx).settings().unwrap();
        settings
            .set("alpha.token", json!("alpha-secret"), Level::Project)
            .unwrap();
        settings
            .set("beta.token", json!("beta-secret"), Level::Project)
            .unwrap();
        assert_eq!(
            native_get(&fx, composed_run(&fx), "alpha.token")
                .await
                .unwrap()["value"],
            "alpha-secret"
        );
        assert!(native_get(&fx, composed_run(&fx), "beta.token")
            .await
            .is_err());
        // Public reads and direct authorized native calls retain their behavior.
        assert_eq!(
            native_get(&fx, composed_run(&fx), "beta.limit")
                .await
                .unwrap()["value"],
            1
        );
        assert_eq!(
            native_get(
                &fx,
                RunContext::core(Actor::Human, Mode::External),
                "beta.token"
            )
            .await
            .unwrap()["value"],
            "beta-secret"
        );
        for value in [None, Some(json!(7))] {
            let result = write(
                target(&fx),
                Invocation {
                    run: composed_run(&fx),
                    idempotency_key: None,
                    confirmed: true,
                },
                "beta.limit".into(),
                value,
                Level::Project,
            );
            assert!(matches!(result, Err(CommandError::Denied { .. })));
        }
        assert_eq!(
            settings.get("beta.limit", None).unwrap().value,
            Some(json!(1))
        );
        write(
            target(&fx),
            Invocation {
                run: composed_run(&fx),
                idempotency_key: None,
                confirmed: true,
            },
            "alpha.limit".into(),
            Some(json!(7)),
            Level::Project,
        )
        .unwrap();
        assert_eq!(
            settings.get("alpha.limit", None).unwrap().value,
            Some(json!(7))
        );
    }
}
