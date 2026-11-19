//! Uniform public operations for every registered setting.

use std::sync::Arc;

use oxplow_config::settings::{Level, SettingsError};
use oxplow_domain::events::schema::{ConfigChanged, ConfigChangedV3};
use oxplow_domain::{Actor, CommandCall, CommandError, Envelope};
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

/// A value its schema refused: the field is where in the value (the first
/// problem), the message every problem in words.
fn value_error(error: SettingsError) -> CommandError {
    let field = match &error {
        SettingsError::ValueRefused(problems) => problems
            .first()
            .map_or("/value".to_owned(), |p| format!("/value{}", p.pointer)),
        _ => "/value".to_owned(),
    };
    CommandError::Invalid {
        field: Some(field),
        message: error.to_string(),
    }
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
            let value = settings
                .snapshot()
                .and_then(|snapshot| snapshot.get(&input.key, input.level))
                .map_err(error)?;
            ctx.run.used_scope("config.read");
            ctx.trace.count("config.read");
            if value.secret {
                ctx.trace.count("config.secret");
                ctx.run.trace().count("config.secret");
            }
            let mut result = serde_json::to_value(&value).expect("setting serializes");
            // An agent never holds a secret: it reads its presence and a
            // mask. An extension's command reading its own (composed, owner
            // checked above) gets the value it needs, whoever ran it.
            if value.secret && ctx.run.actor().is_agent_driven() && ctx.run.composed_by().is_none()
            {
                result["value"] = Value::Null;
                result["masked"] = value
                    .value
                    .as_ref()
                    .map_or(Value::Null, |v| json!(definition.mask.apply(v)));
            }
            Ok(HandlerOutput {
                result,
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
                authorize_writes(&target, &invocation.run, &changes)?;
                write_changes(&target, invocation.actor(), changes, input.level)
            })
        })),
    )
    .with_unrecorded(&["set"]);
    vec![list, get, set, unset, update]
}

fn write(
    target: ConfigTarget,
    invocation: Invocation,
    key: String,
    value: Option<Value>,
    level: Level,
) -> Result<HandlerOutput, CommandError> {
    let changes = vec![(key.clone(), value)];
    authorize_writes(&target, &invocation.run, &changes)?;
    let mut output = write_changes(&target, invocation.actor(), changes, level)?;
    output.result["key"] = json!(key);
    Ok(output)
}

/// A composed run writes only its own extension's settings through the
/// generic commands. A domain action is its own grant, so it has no such
/// check.
fn authorize_writes(
    target: &ConfigTarget,
    run: &oxplow_domain::run::RunContext,
    changes: &[(String, Option<Value>)],
) -> Result<(), CommandError> {
    let settings = target.settings()?;
    for (key, _) in changes {
        let definition = settings.definition(key).map_err(error)?;
        authorize_composer(target, run, &definition, true)?;
    }
    Ok(())
}

/// The one write of registered settings at one level: `config.set`,
/// `config.unset`, `config.update` and every domain action (mark a file
/// generated, choose a provider, …) come here. Each value is checked by its
/// schema (a required extension's switch is `const: true`) and by its own
/// core rule before the file changes. A key whose
/// stored value is the same is not written, logged or undone.
pub(crate) fn write_changes(
    target: &ConfigTarget,
    actor: &Actor,
    changes: Vec<(String, Option<Value>)>,
    level: Level,
) -> Result<HandlerOutput, CommandError> {
    let settings = target.settings()?;
    let snapshot = settings.snapshot().map_err(error)?;
    let layer_values = snapshot.documents().at(level);
    let refused = |message: String| CommandError::Invalid {
        field: Some("/value".into()),
        message,
    };
    let count = changes.len();
    let mut keys = std::collections::BTreeSet::new();
    let mut writes = Vec::new();
    let mut secret = false;
    let mut undoable = true;
    for (key, value) in changes {
        if !keys.insert(key.clone()) {
            return Err(error(SettingsError::DuplicateKey));
        }
        let definition = settings.definition(&key).map_err(error)?;
        if let Some(value) = &value {
            settings.validate(&key, value).map_err(value_error)?;
            oxplow_config::core_settings::check(&key, value).map_err(|e| refused(e.to_string()))?;
        }
        if definition.secret {
            // A secret's stored value is never read back, so it is always
            // written and has no undo.
            secret = true;
            undoable = false;
            writes.push((key, value, None));
            continue;
        }
        let before = layer_values.get(&key).cloned();
        if before == value {
            continue;
        }
        if before
            .as_ref()
            .is_some_and(|value| settings.validate(&key, value).is_err())
        {
            undoable = false;
        }
        writes.push((key, value, before));
    }
    if secret && count != 1 {
        return Err(error(SettingsError::PlaintextSecret));
    }
    if writes.is_empty() {
        return Ok(HandlerOutput {
            result: json!({"level":level,"secret":false,"changed":false}),
            unchanged: true,
            ..Default::default()
        });
    }
    // The typed config the files would make: a write that would leave a
    // project that loads unloadable (an unset exposing a bad hand-edited
    // value) is refused before the file changes. Files that already fail to
    // load are repaired one key at a time, so then nothing is refused here.
    let fallback = target
        .project_dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let loads = |snapshot: &oxplow_config::settings::SettingsSnapshot| -> Result<(), String> {
        let values =
            oxplow_config::core_settings::effective_values(snapshot).map_err(|e| e.to_string())?;
        oxplow_config::core_settings::config_from(&fallback, &values)
            .map(drop)
            .map_err(|e| e.to_string())
    };
    let loaded = loads(&snapshot).is_ok();
    if !secret && loaded {
        let mut levels = Level::PRECEDENCE.map(|at| snapshot.documents().at(at).clone());
        let edited = &mut levels[Level::PRECEDENCE
            .iter()
            .position(|at| *at == level)
            .expect("a level is in the precedence")];
        for (key, value, _) in &writes {
            match value {
                Some(value) => edited.insert(key.clone(), value.clone()),
                None => edited.remove(key),
            };
        }
        let after = settings.snapshot_of(oxplow_config::settings::Documents::new(levels));
        loads(&after).map_err(refused)?;
    }
    // The file is an external system: complete its one atomic replacement
    // before the bus records success, events or undo.
    if secret {
        let (key, value, _) = &writes[0];
        match value {
            Some(value) => settings.set(key, value.clone(), level),
            None => settings.unset(key, level),
        }
        .map_err(error)?;
    } else {
        let changes: Vec<_> = writes
            .iter()
            .map(|(key, value, _)| (key.clone(), value.clone()))
            .collect();
        settings.update(&changes, level).map_err(error)?;
    }
    // A repair of files that already failed to load can leave them still
    // failing (another key is bad): the write stands, and the answer says why.
    let problem = match target.refresh() {
        Ok(()) => None,
        Err(error) if !loaded => Some(error.to_string()),
        Err(error) => return Err(error),
    };
    let mut restore = serde_json::Map::new();
    let mut unset = Vec::new();
    let mut events = Vec::new();
    for (key, value, before) in &writes {
        match before {
            Some(before) => {
                restore.insert(key.clone(), before.clone());
            }
            None => unset.push(key.clone()),
        }
        events.push(
            Envelope::typed::<ConfigChanged>(
                actor.source(),
                &ConfigChangedV3 {
                    key: key.clone(),
                    before: before.clone().unwrap_or(Value::Null),
                    after: if secret {
                        Value::Null
                    } else {
                        value.clone().unwrap_or(Value::Null)
                    },
                    layer: level,
                },
            )
            .with_subject([oxplow_domain::uri::build::config_uri(key)]),
        );
    }
    let inverse = undoable.then(|| match writes.as_slice() {
        [(key, _, before)] => match before {
            Some(value) => CommandCall {
                name: "oxplow.config.set".into(),
                input: json!({"key":key,"value":value,"level":level}),
            },
            None => CommandCall {
                name: "oxplow.config.unset".into(),
                input: json!({"key":key,"level":level}),
            },
        },
        _ => CommandCall {
            name: "oxplow.config.update".into(),
            input: json!({"set":restore,"unset":unset,"level":level}),
        },
    });
    Ok(HandlerOutput {
        result: match &problem {
            None => json!({"level":level,"secret":secret,"changed":true,"loads":true}),
            Some(problem) => json!({
                "level":level,"secret":secret,"changed":true,"loads":false,"problem":problem
            }),
        },
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

    #[tokio::test]
    async fn a_required_extension_cannot_be_turned_off() {
        let fx = services_with_thread().await;
        let key = "oxplow.extensions.oxplow-foundation.enabled";
        let refused = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                "oxplow.config.set",
                json!({"key":key,"value":false,"level":"project"}),
                false,
            )
            .await;
        assert!(refused.is_err(), "{refused:?}");
        let project = fx._dir.path().join(".oxplow/project.yaml");
        assert!(!std::fs::read_to_string(&project)
            .unwrap_or_default()
            .contains(key));
        // The switch is registered: its schema, not an unknown key, refused it.
        fx.svc
            .commands
            .run(
                &Actor::Human,
                "oxplow.config.set",
                json!({"key":key,"value":true,"level":"project"}),
                false,
            )
            .await
            .unwrap();
        // Once the switch is in a file it is still the required one.
        let again = fx
            .svc
            .commands
            .run(
                &Actor::Human,
                "oxplow.config.set",
                json!({"key":key,"value":false,"level":"local"}),
                false,
            )
            .await;
        assert!(again.is_err(), "{again:?}");
    }

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
      limit: { type: integer, default: 1 }
      token: { type: string, x-secret: true }
";

    fn target(fx: &ThreadFixture) -> ConfigTarget {
        ConfigTarget {
            config: fx.svc.config.clone(),
            project_dir: fx.svc.layout.project_dir.clone(),
            global_dir: fx.svc.layout.state_dir.join("global-config"),
            secrets: fx.svc.secrets.clone(),
            catalog: Some(fx.svc.extension_catalog.clone()),
            events: fx.svc.events.clone(),
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

    /// An agent reading a secret gets its presence and a mask, never the
    /// value; a person, and an extension reading its own, get the value.
    /// An ordinary setting reads the same for both.
    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_reads_a_secret_masked() {
        let fx = services_with_thread().await;
        declare(fx._dir.path(), "alpha", CONFIGURATION);
        reconcile(&fx).await;
        let settings = target(&fx).settings().unwrap();
        settings
            .set(
                "alpha.token",
                json!("sk-ant-api03-abcdWXYZ"),
                Level::Project,
            )
            .unwrap();
        let agent = || {
            RunContext::core(
                Actor::Agent {
                    session_id: None,
                    thread_id: None,
                    stream_id: None,
                },
                Mode::External,
            )
        };
        let read = native_get(&fx, agent(), "alpha.token").await.unwrap();
        assert_eq!(read["value"], Value::Null);
        assert_eq!(read["masked"], "••••WXYZ");
        assert_eq!(read["isSet"], true);
        assert_eq!(read["secret"], true);
        assert_eq!(read["origin"], "project");
        assert!(!read.to_string().contains("abcd"));
        assert_eq!(
            native_get(
                &fx,
                RunContext::core(Actor::Human, Mode::External),
                "alpha.token"
            )
            .await
            .unwrap()["value"],
            "sk-ant-api03-abcdWXYZ"
        );
        assert_eq!(
            native_get(&fx, composed_run(&fx), "alpha.token")
                .await
                .unwrap()["value"],
            "sk-ant-api03-abcdWXYZ"
        );
        let plain = native_get(&fx, agent(), "alpha.limit").await.unwrap();
        assert_eq!(plain["value"], 1);
        assert!(plain.get("masked").is_none());
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
                    .snapshot()
                    .and_then(|snapshot| snapshot.inspect(key, Some(Level::Project)))
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
            settings
                .snapshot()
                .and_then(|snapshot| snapshot.get("beta.limit", None))
                .unwrap()
                .value,
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
            settings
                .snapshot()
                .and_then(|snapshot| snapshot.get("alpha.limit", None))
                .unwrap()
                .value,
            Some(json!(7))
        );
    }
}
