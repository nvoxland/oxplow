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

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateInput {
    level: Level,
    #[serde(default)]
    set: std::collections::BTreeMap<String, Value>,
    #[serde(default)]
    unset: Vec<String>,
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
pub(super) fn authorize_composer(
    target: &ConfigTarget,
    run: &oxplow_domain::run::RunContext,
    definition: &oxplow_config::settings::SettingDefinition,
    writing: bool,
) -> Result<(), CommandError> {
    if run.composed_by().is_none() || (!writing && !definition.secret) {
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
    let unset_target = target.clone();
    let unset = Op::new(
        "config.write",
        "unset",
        schema::<UnsetInput>(),
        true,
        Handler::External(Arc::new(move |invocation, input| {
            let target = unset_target.clone();
            Box::pin(async move {
                let input: UnsetInput = parse(input)?;
                write(target, invocation, input.key, None, input.level)
            })
        })),
    );
    let update = Op::new(
        "config.write",
        "update",
        schema::<UpdateInput>(),
        true,
        Handler::External(Arc::new(move |invocation, input| {
            let target = target.clone();
            Box::pin(async move {
                let input: UpdateInput = parse(input)?;
                let mut changes: Vec<_> = input
                    .set
                    .into_iter()
                    .map(|(key, value)| (key, Some(value)))
                    .collect();
                changes.extend(input.unset.into_iter().map(|key| (key, None)));
                write_changes(target, invocation, changes, input.level)
            })
        })),
    )
    .with_unrecorded(&["set"]);
    vec![list, get, set, unset, update]
}

pub(super) fn validate_change(key: &str, value: Option<&Value>) -> Result<(), CommandError> {
    if key == "oxplow.extensions.oxplow-foundation.enabled" && value == Some(&Value::Bool(false)) {
        return Err(CommandError::Invalid {
            field: Some("/value".into()),
            message: "oxplow-foundation is required and cannot be disabled".into(),
        });
    }
    Ok(())
}

fn write(
    target: ConfigTarget,
    invocation: Invocation,
    key: String,
    value: Option<Value>,
    level: Level,
) -> Result<HandlerOutput, CommandError> {
    let mut output = write_changes(target, invocation, vec![(key.clone(), value)], level)?;
    output.result["key"] = json!(key);
    Ok(output)
}

fn write_changes(
    target: ConfigTarget,
    invocation: Invocation,
    changes: Vec<(String, Option<Value>)>,
    level: Level,
) -> Result<HandlerOutput, CommandError> {
    let settings = target.settings()?;
    let mut keys = std::collections::BTreeSet::new();
    let mut before_values = Vec::new();
    let layer_values = settings.layer(level).map_err(error)?;
    let mut has_secret = false;
    let mut undoable = true;
    for (key, value) in &changes {
        if !keys.insert(key.clone()) {
            return Err(error(SettingsError::DuplicateKey));
        }
        validate_change(key, value.as_ref())?;
        let definition = settings.definition(key).map_err(error)?;
        authorize_composer(&target, &invocation.run, &definition, true)?;
        has_secret |= definition.secret;
        let before = if definition.secret {
            undoable = false;
            None
        } else {
            layer_values.get(key).cloned()
        };
        if before
            .as_ref()
            .is_some_and(|value| settings.validate(key, value).is_err())
        {
            undoable = false;
        }
        before_values.push(before);
    }
    if has_secret && changes.len() != 1 {
        return Err(error(SettingsError::PlaintextSecret));
    }
    super::config_commands::preflight(&target, &settings, &changes, level)?;
    if has_secret {
        let (key, value) = &changes[0];
        match value {
            Some(value) => settings.set(key, value.clone(), level),
            None => settings.unset(key, level),
        }
        .map_err(error)?;
    } else {
        settings.update(&changes, level).map_err(error)?;
    }
    target.refresh()?;
    let mut restore = serde_json::Map::new();
    let mut unset = Vec::new();
    let mut events = Vec::new();
    for ((key, value), before) in changes.iter().zip(before_values) {
        if let Some(value) = &before {
            restore.insert(key.clone(), value.clone());
        } else {
            unset.push(key.clone());
        }
        events.push(
            Envelope::typed::<ConfigChanged>(
                invocation.run.actor().source(),
                &ConfigChangedV2 {
                    key: key.clone(),
                    before: before.unwrap_or(Value::Null),
                    after: if has_secret {
                        Value::Null
                    } else {
                        value.clone().unwrap_or(Value::Null)
                    },
                    layer: match level {
                        Level::Local => ConfigLayer::Local,
                        Level::Project => ConfigLayer::Project,
                        Level::Global => ConfigLayer::Global,
                    },
                },
            )
            .with_subject([oxplow_domain::uri::build::config_uri(key)]),
        );
    }
    let inverse = undoable.then(|| {
        if changes.len() == 1 {
            let key = &changes[0].0;
            match restore.get(key) {
                Some(value) => CommandCall {
                    name: "oxplow.config.set".into(),
                    input: json!({"key":key,"value":value,"level":level}),
                },
                None => CommandCall {
                    name: "oxplow.config.unset".into(),
                    input: json!({"key":key,"level":level}),
                },
            }
        } else {
            CommandCall {
                name: "oxplow.config.update".into(),
                input: json!({"set":restore,"unset":unset,"level":level}),
            }
        }
    });
    Ok(HandlerOutput {
        result: json!({"level":level,"secret":has_secret,"changed":true}),
        inverse,
        events,
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

    #[cfg(unix)]
    #[tokio::test]
    async fn failed_batch_replacement_leaves_all_keys_and_success_history_unchanged() {
        use std::os::unix::fs::PermissionsExt;
        let fx = services_with_thread().await;
        let values =
            json!({"oxplow.snapshots.retentionDays":14,"oxplow.generated.exclude":["first"]});
        fx.svc
            .commands
            .run(
                &Actor::Human,
                "oxplow.config.update",
                json!({"level":"project","set":values}),
                false,
            )
            .await
            .unwrap();
        let path = fx._dir.path().join(".oxplow/project.yaml");
        let before = std::fs::read(&path).unwrap();
        let directory = path.parent().unwrap();
        let permissions = std::fs::metadata(directory).unwrap().permissions();
        std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o500)).unwrap();
        let result=fx.svc.commands.run(&Actor::Human,"oxplow.config.update",json!({"level":"project","set":{"oxplow.snapshots.retentionDays":3,"oxplow.generated.exclude":["second"]}}),false).await;
        std::fs::set_permissions(directory, permissions).unwrap();
        assert!(
            result.is_err(),
            "a failed replacement is returned to the caller"
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let successful=fx.svc.db.read(|tx|tx.query_row("SELECT count(*) FROM command_audit WHERE command='oxplow.config.update' AND outcome='ok'",[],|row|row.get::<_,i64>(0)).map_err(oxplow_db::map_sql_err)).await.unwrap();
        assert_eq!(
            successful, 1,
            "the failed batch must not gain a successful audit"
        );
    }

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
    async fn aggregate_ai_provider_settings_are_not_registered() {
        let fx = services_with_thread().await;
        assert!(target(&fx)
            .settings()
            .unwrap()
            .definition("oxplow.ai.providers")
            .is_err());
        let result = write(
            target(&fx),
            Invocation {
                run: RunContext::core(Actor::Human, Mode::External),
                idempotency_key: None,
                confirmed: true,
            },
            "oxplow.ai.providers".into(),
            Some(json!([])),
            Level::Project,
        );
        assert!(result.is_err());
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
        for (key, value) in [
            ("oxplow.snapshots.retentionDays", json!(7)),
            ("oxplow.ai.openai.apiKey", json!("must-not-save")),
        ] {
            for value in [None, Some(value)] {
                let result = write(
                    target(&fx),
                    Invocation {
                        run: composed_run(&fx),
                        idempotency_key: None,
                        confirmed: true,
                    },
                    key.into(),
                    value,
                    Level::Project,
                );
                assert!(
                    matches!(result, Err(CommandError::Denied { .. })),
                    "composed write to {key} must be denied"
                );
            }
            assert!(
                !target(&fx)
                    .settings()
                    .unwrap()
                    .inspect(key, Some(Level::Project))
                    .unwrap()
                    .is_set
            );
        }
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
