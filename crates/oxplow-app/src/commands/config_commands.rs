//! `config.*`: `.oxplow/project.yaml` as commands (`.context/commands.md`).
//!
//! `config.list_keys` / `config.get` describe the file through the key
//! registry (`oxplow_config::keys`); `config.set` / `config.unset` change
//! one key, taking the new document through the loader's own validation,
//! writing the file, updating the in-memory config, and logging
//! `config.changed@1 { key, before, after }` with an inverse that restores
//! the prior value. A human-only key (`ai`, `agents`, `lsp`, …) asks for
//! confirmation per input, so an agent gets `NeedsConfirmation` while a
//! person's confirmed call goes through.
//!
//! The handlers are `Tx`: the file write happens inside the bus's
//! transaction so a failed write leaves no audit row or event; the
//! in-memory config is replaced only once the file is written.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use oxplow_config::keys::{config_key, config_keys, key_value, with_key, ConfigKey};
use oxplow_config::{write_project_config, OxplowConfig};
use oxplow_domain::events::schema::{ConfigChanged, ConfigChangedV1};
use oxplow_domain::{
    Actor, Atomicity, CommandCall, CommandError, CommandSpec, Confirm, Envelope, InputValidator,
    Invokers, Lifecycle,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{Command, Handler, HandlerOutput};
use crate::events::EventBus;
use crate::OxplowEvent;

pub const LIST_KEYS: &str = "config.list_keys";
pub const GET: &str = "config.get";
pub const SET: &str = "config.set";
pub const UNSET: &str = "config.unset";

/// What the config commands act on.
#[derive(Clone)]
pub struct ConfigTarget {
    pub config: Arc<RwLock<OxplowConfig>>,
    pub project_dir: PathBuf,
    pub events: EventBus,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoInput {}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyInput {
    /// A `.oxplow/project.yaml` key (`zones`, `metricRetentionDays`, …).
    pub key: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetInput {
    /// A `.oxplow/project.yaml` key.
    pub key: String,
    /// The new value, in the key's own shape (see `config.list_keys`).
    pub value: Value,
}

/// One key as `config.list_keys` / `config.get` report it.
#[derive(Debug, Serialize, Deserialize)]
pub struct KeyReport {
    #[serde(flatten)]
    pub key: ConfigKey,
    /// Whether the file sets it (else the value is the default).
    pub set: bool,
    /// The value as written to the file; `null` when unset.
    pub value: Value,
}

fn schema_of<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).expect("schema serializes")
}

fn spec(name: &str, summary: &str, input_schema: Value, undoable: bool) -> CommandSpec {
    CommandSpec {
        name: name.into(),
        summary: summary.into(),
        input_schema,
        invokers: Invokers::ALL,
        confirm: Confirm::Never,
        undoable,
        lifecycle: Lifecycle::Stable,
        atomicity: Atomicity::Tx,
    }
}

fn parse<T: for<'de> Deserialize<'de>>(input: Value) -> Result<T, CommandError> {
    serde_json::from_value(input).map_err(|e| CommandError::Invalid {
        field: None,
        message: e.to_string(),
    })
}

fn known_key(key: &str) -> Result<ConfigKey, CommandError> {
    config_key(key).ok_or_else(|| CommandError::Invalid {
        field: Some("/key".into()),
        message: format!("`{key}` is not a project.yaml key; see config.list_keys"),
    })
}

/// A human-only key needs a person's confirmation; the rest don't.
fn confirm_for_key(input: &Value) -> Confirm {
    match input
        .get("key")
        .and_then(|k| k.as_str())
        .and_then(config_key)
    {
        Some(k) if k.human_only => Confirm::Always,
        _ => Confirm::Never,
    }
}

fn report(target: &ConfigTarget, cfg: &OxplowConfig, key: ConfigKey) -> KeyReport {
    let value = key_value(cfg, &target.project_dir, &key.key);
    KeyReport {
        set: value.is_some(),
        value: value.unwrap_or(Value::Null),
        key,
    }
}

/// Set (`Some`) or unset (`None`) `key`: validate, write the file, swap the
/// in-memory config, and describe the change.
fn change(
    target: &ConfigTarget,
    actor: &Actor,
    key: &str,
    value: Option<Value>,
) -> Result<HandlerOutput, CommandError> {
    let spec = known_key(key)?;
    if let Some(v) = &value {
        if v.is_null() {
            return Err(CommandError::Invalid {
                field: Some("/value".into()),
                message: format!("`{key}` cannot be set to null; use config.unset"),
            });
        }
        InputValidator::compile(&spec.schema)?
            .check(v)
            .map_err(|e| match e {
                CommandError::Invalid { field, message } => CommandError::Invalid {
                    field: Some(format!("/value{}", field.unwrap_or_default())),
                    message,
                },
                other => other,
            })?;
    }
    let mut guard = target.config.write().unwrap_or_else(|e| e.into_inner());
    let before = key_value(&guard, &target.project_dir, key);
    let next = with_key(&guard, &target.project_dir, key, value.as_ref()).map_err(|e| {
        CommandError::Invalid {
            field: Some("/value".into()),
            message: e.to_string(),
        }
    })?;
    let after = key_value(&next, &target.project_dir, key);
    if before == after {
        // Nothing to write, log or undo.
        return Ok(HandlerOutput {
            result: json!({ "key": key, "before": before, "after": after, "changed": false }),
            ..HandlerOutput::default()
        });
    }
    write_project_config(&target.project_dir, &next).map_err(|e| CommandError::Failed {
        message: format!("writing project.yaml: {e}"),
    })?;
    *guard = next;
    drop(guard);
    let inverse = match &before {
        Some(prev) => CommandCall {
            name: SET.into(),
            input: json!({ "key": key, "value": prev }),
        },
        None => CommandCall {
            name: UNSET.into(),
            input: json!({ "key": key }),
        },
    };
    let event = Envelope::typed::<ConfigChanged>(
        actor.source(),
        &ConfigChangedV1 {
            key: key.to_string(),
            before: before.clone().unwrap_or(Value::Null),
            after: after.clone().unwrap_or(Value::Null),
        },
    )
    .with_subject([format!("config:{key}")]);
    let events = target.events.clone();
    Ok(HandlerOutput {
        result: json!({ "key": key, "before": before, "after": after, "changed": true }),
        inverse: Some(inverse),
        events: vec![event],
        after_commit: Some(Box::new(move || events.emit(OxplowEvent::ConfigChanged))),
    })
}

/// The four `config.*` commands over `target`.
pub fn commands(target: ConfigTarget) -> Vec<Command> {
    let list = {
        let t = target.clone();
        Command::new(
            spec(
                LIST_KEYS,
                "Every .oxplow/project.yaml key with its doc, value schema, current value and \
                 whether only a person may set it.",
                schema_of::<NoInput>(),
                false,
            ),
            Handler::Tx(Arc::new(move |_conn, _actor, input| {
                parse::<NoInput>(input)?;
                let cfg = t.config.read().unwrap_or_else(|e| e.into_inner()).clone();
                let keys: Vec<KeyReport> = config_keys()
                    .into_iter()
                    .map(|k| report(&t, &cfg, k))
                    .collect();
                Ok(HandlerOutput {
                    result: serde_json::to_value(keys).expect("reports serialize"),
                    ..HandlerOutput::default()
                })
            })),
        )
        .expect("config.list_keys registers")
    };
    let get = {
        let t = target.clone();
        Command::new(
            spec(
                GET,
                "One .oxplow/project.yaml key: its doc, value schema and current value.",
                schema_of::<KeyInput>(),
                false,
            ),
            Handler::Tx(Arc::new(move |_conn, _actor, input| {
                let input: KeyInput = parse(input)?;
                let key = known_key(&input.key)?;
                let cfg = t.config.read().unwrap_or_else(|e| e.into_inner()).clone();
                Ok(HandlerOutput {
                    result: serde_json::to_value(report(&t, &cfg, key)).expect("report serializes"),
                    ..HandlerOutput::default()
                })
            })),
        )
        .expect("config.get registers")
    };
    let set = {
        let t = target.clone();
        Command::new(
            spec(
                SET,
                "Set one .oxplow/project.yaml key. The value is validated against the key's \
                 schema and the file's own rules, written to the file, and logged as \
                 config.changed; undo restores the prior value. Human-only keys need a \
                 person's confirmation.",
                schema_of::<SetInput>(),
                true,
            ),
            Handler::Tx(Arc::new(move |_conn, actor, input| {
                let input: SetInput = parse(input)?;
                change(&t, actor, &input.key, Some(input.value))
            })),
        )
        .expect("config.set registers")
        .with_confirm_for(Arc::new(confirm_for_key))
    };
    let unset = {
        let t = target;
        Command::new(
            spec(
                UNSET,
                "Remove one .oxplow/project.yaml key so it returns to its default; logged as \
                 config.changed, undo restores it.",
                schema_of::<KeyInput>(),
                true,
            ),
            Handler::Tx(Arc::new(move |_conn, actor, input| {
                let input: KeyInput = parse(input)?;
                change(&t, actor, &input.key, None)
            })),
        )
        .expect("config.unset registers")
        .with_confirm_for(Arc::new(confirm_for_key))
    };
    vec![list, get, set, unset]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_policy::AgentPolicy;
    use crate::commands::CommandBus;
    use crate::event_pump::EventPump;
    use oxplow_config::load_project_config;
    use oxplow_db::{Database, SqliteEventLogStore};
    use oxplow_domain::{EventSchemaRegistry, ThreadId};

    fn setup(initial_yaml: Option<&str>) -> (tempfile::TempDir, ConfigTarget, CommandBus) {
        let dir = tempfile::tempdir().unwrap();
        if let Some(yaml) = initial_yaml {
            std::fs::create_dir_all(dir.path().join(".oxplow")).unwrap();
            std::fs::write(dir.path().join(".oxplow/project.yaml"), yaml).unwrap();
        }
        let config = Arc::new(RwLock::new(load_project_config(dir.path()).unwrap()));
        let target = ConfigTarget {
            config,
            project_dir: dir.path().to_path_buf(),
            events: EventBus::new(),
        };
        let db = Database::in_memory();
        let log = SqliteEventLogStore::new(db.clone(), Arc::new(EventSchemaRegistry::core()));
        let pump = Arc::new(EventPump::new(db.clone(), log.clone(), vec![]));
        let bus = CommandBus::new(db, log, Arc::new(AgentPolicy::default()), pump);
        for c in commands(target.clone()) {
            bus.register(c).unwrap();
        }
        (dir, target, bus)
    }

    fn agent() -> Actor {
        Actor::Agent {
            thread_id: Some(ThreadId::new(1)),
            stream_id: None,
        }
    }

    fn file(dir: &tempfile::TempDir) -> String {
        std::fs::read_to_string(dir.path().join(".oxplow/project.yaml")).unwrap_or_default()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn list_and_get_describe_every_key_with_its_state() {
        let (_dir, _t, bus) =
            setup(Some("zones:\n  - match: src/**\n    zone: core\n")).await_ready();
        let out = bus
            .run(&agent(), LIST_KEYS, json!({}), false)
            .await
            .unwrap();
        let keys = out.result.as_array().unwrap();
        let zones = keys.iter().find(|k| k["key"] == "zones").unwrap();
        assert_eq!(zones["set"], true);
        assert_eq!(zones["value"][0]["zone"], "core");
        assert_eq!(zones["human_only"], false);
        let ai = keys.iter().find(|k| k["key"] == "ai").unwrap();
        assert_eq!(ai["human_only"], true);
        assert_eq!(ai["set"], false);
        assert!(ai["schema"].get("$defs").is_some());
        let got = bus
            .run(
                &Actor::Human,
                GET,
                json!({"key": "metricRetentionDays"}),
                false,
            )
            .await
            .unwrap();
        assert_eq!(got.result["set"], false);
        assert_eq!(got.result["value"], Value::Null);
        let err = bus
            .run(&Actor::Human, GET, json!({"key": "nope"}), false)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f == "/key"),
            "{err:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_agent_sets_zones_and_undo_restores_the_file() {
        let (dir, target, bus) = setup(None).await_ready();
        let out = bus
            .run(
                &agent(),
                SET,
                json!({"key": "zones", "value": [{"match": "src/**", "zone": "core"}]}),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["changed"], true);
        assert_eq!(out.result["before"], Value::Null);
        assert_eq!(out.result["after"][0]["zone"], "core");
        assert!(file(&dir).contains("zone: core"), "{}", file(&dir));
        assert_eq!(target.config.read().unwrap().zones.len(), 1);
        // The change was logged with the command that caused it.
        let events = bus.log_for_tests().read_after(0, 10).await.unwrap();
        let changed = events
            .iter()
            .find(|e| e.envelope.event_type == "config.changed")
            .unwrap();
        assert_eq!(changed.envelope.payload["key"], "zones");
        assert_eq!(changed.envelope.payload["before"], Value::Null);
        assert_eq!(changed.envelope.source, "agent:thr1");
        assert_eq!(changed.envelope.subject, vec!["config:zones"]);
        assert!(changed.envelope.cause.is_some());
        // Undo: the inverse of a first set is an unset.
        assert_eq!(out.inverse.as_ref().unwrap().name, UNSET);
        bus.undo(&agent(), out.audit_id, false).await.unwrap();
        assert!(!file(&dir).contains("zones"), "{}", file(&dir));
        assert!(target.config.read().unwrap().zones.is_empty());
        // A no-op set changes nothing and logs nothing new.
        let before = bus.log_for_tests().read_after(0, 100).await.unwrap().len();
        let noop = bus
            .run(&agent(), UNSET, json!({"key": "zones"}), false)
            .await
            .unwrap();
        assert_eq!(noop.result["changed"], false);
        assert!(noop.inverse.is_none());
        // (`command.executed` is still logged for the run itself.)
        assert_eq!(
            bus.log_for_tests().read_after(0, 100).await.unwrap().len(),
            before + 1
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_human_only_key_needs_a_persons_confirmation() {
        let (dir, target, bus) = setup(None).await_ready();
        let value = json!({"roles": {"main": {"provider": "anthropic", "model": "claude"}}});
        let err = bus
            .run(&agent(), SET, json!({"key": "ai", "value": value}), true)
            .await
            .unwrap_err();
        assert!(
            matches!(err, CommandError::NeedsConfirmation { .. }),
            "{err:?}"
        );
        assert!(
            !file(&dir).contains("ai"),
            "nothing written: {}",
            file(&dir)
        );
        assert!(target.config.read().unwrap().ai_roles.is_empty());
        // The person confirms.
        let err = bus
            .run(
                &Actor::Human,
                SET,
                json!({"key": "ai", "value": value}),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::NeedsConfirmation { .. }));
        bus.run(
            &Actor::Human,
            SET,
            json!({"key": "ai", "value": value}),
            true,
        )
        .await
        .unwrap();
        assert_eq!(
            target.config.read().unwrap().ai_roles["main"].model,
            "claude"
        );
        assert!(file(&dir).contains("anthropic"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn values_are_validated_by_schema_and_by_the_loaders_rules() {
        let (dir, _t, bus) = setup(None).await_ready();
        let err = bus
            .run(
                &agent(),
                SET,
                json!({"key": "metricRetentionDays", "value": "soon"}),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CommandError::Invalid { field: Some(f), .. } if f.starts_with("/value")),
            "{err:?}"
        );
        let err = bus
            .run(
                &agent(),
                SET,
                json!({"key": "zones", "value": [{"match": "x", "zone": "other"}]}),
                false,
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("reserved"), "{err}");
        let err = bus
            .run(&agent(), SET, json!({"key": "zones", "value": null}), false)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("config.unset"), "{err}");
        assert!(file(&dir).is_empty());
    }

    /// The old `MANAGED_KEYS` list missed `metricRetentionDays` (and the
    /// detail knobs, and `iconTint`): the on-disk value came back as an
    /// "extra" and overwrote the one just set. The registry closes that.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_previously_unmanaged_key_round_trips_instead_of_reverting() {
        let (dir, target, bus) =
            setup(Some("metricRetentionDays: 7\niconTint: '#abc'\n")).await_ready();
        bus.run(
            &agent(),
            SET,
            json!({"key": "metricRetentionDays", "value": 30}),
            false,
        )
        .await
        .unwrap();
        bus.run(
            &agent(),
            SET,
            json!({"key": "iconTint", "value": "#c2410c"}),
            false,
        )
        .await
        .unwrap();
        let reloaded = load_project_config(dir.path()).unwrap();
        assert_eq!(reloaded.metric_retention_days, 30);
        assert_eq!(reloaded.icon_tint.as_deref(), Some("#c2410c"));
        assert_eq!(target.config.read().unwrap().metric_retention_days, 30);
    }

    /// `setup` is sync; this lets the tests read as one chain.
    trait AwaitReady {
        fn await_ready(self) -> Self;
    }
    impl<T> AwaitReady for T {
        fn await_ready(self) -> Self {
            self
        }
    }
}
