//! Domain settings actions compose the same registered storage used by config.*.
//! Runtime document fields are translated to independently registered keys.

use crate::commands::ops::Op;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use oxplow_config::settings::Level;
use oxplow_config::OxplowConfig;
use oxplow_domain::events::schema::{ConfigChanged, ConfigChangedV3, ConfigLayer};
use oxplow_domain::{Actor, CommandCall, CommandError, Envelope};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::util::{parse, schema};
use super::{Handler, HandlerOutput};
use crate::events::EventBus;
use crate::OxplowEvent;

pub const LIST_KEYS: &str = "oxplow.config.list_keys";
pub const GET: &str = "oxplow.config.get";
pub const SET: &str = "oxplow.config.set";
pub const UNSET: &str = "oxplow.config.unset";

/// What the config commands act on.
#[derive(Clone)]
pub struct ConfigTarget {
    pub config: Arc<RwLock<OxplowConfig>>,
    pub global_dir: PathBuf,
    pub secrets: Arc<dyn oxplow_ai::secrets::SecretStore>,
    pub catalog: Option<Arc<crate::extension::extension_catalog::ExtensionCatalog>>,
    pub project_dir: PathBuf,
    pub events: EventBus,
    /// Woken after each swap of the in-memory config.
    pub applied: Arc<tokio::sync::Notify>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoInput {}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MarkGeneratedInput {
    /// The file or folder, by the name the project's `generated` list
    /// holds it by (`target`, `dist`, `*.min.js`).
    pub entry: String,
    /// Mark it generated (the default: left out of snapshots and the
    /// change views), or, `false`, take the mark off.
    #[serde(default = "mark")]
    pub generated: bool,
}

fn mark() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChooseProviderInput {
    /// The capability (`snapshots`, `knowledge`, …).
    pub capability: String,
    /// The implementation to use; absent or blank: the default (the
    /// project's, for a personal choice).
    #[serde(default)]
    pub provider: Option<String>,
    /// `project` (the default: shared in `.oxplow/project.yaml`) or
    /// `personal` (just for this person).
    #[serde(default)]
    pub level: Layer,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChooseReplacementInput {
    /// The core sub-component an extension replaces (`task.board`).
    pub target: String,
    /// Whose shows: an extension's name, or `oxplow` for oxplow's own.
    /// Leave it out for the default.
    #[serde(default)]
    pub whose: Option<String>,
}

/// Which layer a config write goes to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    #[default]
    Project,
    Local,
    Global,
}

impl Layer {
    fn level(self) -> Level {
        match self {
            Self::Project => Level::Project,
            Self::Local => Level::Local,
            Self::Global => Level::Global,
        }
    }
    fn logged(self) -> ConfigLayer {
        match self {
            Self::Project => ConfigLayer::Project,
            Self::Local => ConfigLayer::Local,
            Self::Global => ConfigLayer::Global,
        }
    }
    /// Resolve an action's starting value at its chosen level or below.
    /// Editing a shared map must never copy a local override into Git.
    pub(super) fn value(
        self,
        store: &oxplow_config::settings::SettingsStore,
        key: &str,
    ) -> Result<Option<Value>, CommandError> {
        let key = if key.starts_with("oxplow.") {
            key.to_owned()
        } else {
            oxplow_config::core_settings::canonical_key(key)
        };
        let levels: &[Level] = match self {
            Self::Local => &Level::PRECEDENCE,
            Self::Project => &[Level::Project, Level::Global],
            Self::Global => &[Level::Global],
        };
        let failed = |error: oxplow_config::settings::SettingsError| CommandError::Invalid {
            field: Some("/key".into()),
            message: error.to_string(),
        };
        for level in levels {
            if let Some(value) = store.get(&key, Some(*level)).map_err(failed)?.value {
                return Ok(Some(value));
            }
        }
        Ok(store.definition(&key).map_err(failed)?.default)
    }
}

impl ConfigTarget {
    pub(crate) fn settings(&self) -> Result<oxplow_config::settings::SettingsStore, CommandError> {
        let extensions = self
            .catalog
            .as_ref()
            .map(|catalog| catalog.unpinned(&self.project_dir, "settings registry"));
        crate::settings::store(
            &self.project_dir,
            &self.global_dir,
            self.secrets.clone(),
            extensions.as_deref().map(|v| v.as_slice()).unwrap_or(&[]),
        )
        .map_err(|e| CommandError::Failed {
            message: e.to_string(),
        })
    }
    pub(crate) fn refresh(&self) -> Result<(), CommandError> {
        let fresh = oxplow_config::load_project_config_with_global(
            &self.project_dir,
            Some(&self.global_dir),
        )
        .map_err(|e| CommandError::Failed {
            message: e.to_string(),
        })?;
        *self.config.write().unwrap_or_else(|e| e.into_inner()) = fresh;
        self.applied.notify_waiters();
        self.events.emit(OxplowEvent::ConfigChanged);
        Ok(())
    }
}

/// Validate the whole typed core projection before any file write.
pub(super) fn preflight(
    target: &ConfigTarget,
    store: &oxplow_config::settings::SettingsStore,
    changes: &[(String, Option<Value>)],
    level: Level,
) -> Result<(), CommandError> {
    let fail = |e: oxplow_config::settings::SettingsError| CommandError::Invalid {
        field: Some("/value".into()),
        message: e.to_string(),
    };
    let mut layers = std::collections::BTreeMap::new();
    for at in Level::PRECEDENCE {
        let mut values = store.layer(at).map_err(fail)?;
        if at == level {
            for (key, value) in changes {
                if let Some(value) = value {
                    values.insert(key.clone(), value.clone());
                } else {
                    values.remove(key);
                }
            }
        }
        layers.insert(format!("{at:?}"), values);
    }
    let fallback = target
        .project_dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let validate_projection = |precedence: &[Level]| -> Result<(), CommandError> {
        let mut effective = std::collections::BTreeMap::new();
        let definitions = store
            .registry
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .definitions()
            .filter(|definition| definition.owner == "core" && !definition.secret)
            .cloned()
            .collect::<Vec<_>>();
        for definition in definitions {
            for at in precedence {
                if let Some(value) = layers[&format!("{at:?}")].get(&definition.key) {
                    store.validate(&definition.key, value).map_err(fail)?;
                    effective.insert(definition.key, value.clone());
                    break;
                }
            }
        }
        oxplow_config::parse_project_config(
            oxplow_config::core_settings::document_from_values(&fallback, &effective),
            &fallback,
        )
        .map_err(|error| CommandError::Invalid {
            field: Some("/value".into()),
            message: error.to_string(),
        })?;
        Ok(())
    };
    // A valid local value must not hide an invalid shared override. Validate
    // what the edited level would expose without higher overrides, as well
    // as the effective combination the running project will use.
    if level != Level::Local {
        let selected: Vec<_> = Level::PRECEDENCE
            .into_iter()
            .skip_while(|at| *at != level)
            .collect();
        validate_projection(&selected)?;
    }
    validate_projection(&Level::PRECEDENCE)?;
    Ok(())
}

/// Translate a domain action into registered settings, preserving canonical undo.
pub(crate) fn change(
    target: &ConfigTarget,
    actor: &Actor,
    field: &str,
    value: Option<Value>,
    layer: Layer,
) -> Result<HandlerOutput, CommandError> {
    let store = target.settings()?;
    let fallback = target
        .project_dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let bindings = oxplow_config::core_settings::bindings(&fallback);
    let canonical = if field.starts_with("oxplow.") {
        field.to_owned()
    } else {
        oxplow_config::core_settings::canonical_key(field)
    };
    let changes = if store.definition(&canonical).is_ok() {
        // A registered key (including a dotted runtime leaf) changes only
        // that setting. Sibling options retain their own level and origin.
        vec![(canonical, value)]
    } else {
        let keys: Vec<_> = bindings
            .iter()
            .filter(|binding| binding.path.first().is_some_and(|path| path == field))
            .map(|binding| binding.definition.key.clone())
            .collect();
        if keys.is_empty() {
            return Err(CommandError::Invalid {
                field: Some("/key".into()),
                message: "unknown setting".into(),
            });
        }
        let values = value
            .as_ref()
            .map(|value| {
                oxplow_config::core_settings::values_for_runtime_field(&fallback, field, value)
            })
            .unwrap_or_default();
        keys.into_iter()
            .map(|key| {
                let value = values.get(&key).cloned();
                (key, value)
            })
            .collect()
    };
    let fail = |e: oxplow_config::settings::SettingsError| CommandError::Invalid {
        field: Some("/value".into()),
        message: e.to_string(),
    };
    for (key, value) in &changes {
        if let Some(v) = value {
            store.validate(key, v).map_err(fail)?;
        }
    }
    preflight(target, &store, &changes, layer.level())?;
    let mut inverses = Vec::new();
    let mut events = Vec::new();
    let mut writes = Vec::new();
    for (key, value) in changes {
        let before = store.get(&key, Some(layer.level())).map_err(fail)?.value;
        if before == value {
            continue;
        }
        inverses.push(match &before {
            Some(v) => CommandCall {
                name: SET.into(),
                input: json!({"key":key,"value":v,"level":layer.level()}),
            },
            None => CommandCall {
                name: UNSET.into(),
                input: json!({"key":key,"level":layer.level()}),
            },
        });
        events.push(
            Envelope::typed::<ConfigChanged>(
                actor.source(),
                &ConfigChangedV3 {
                    key: key.clone(),
                    before: before.unwrap_or(Value::Null),
                    after: value.clone().unwrap_or(Value::Null),
                    layer: layer.logged(),
                },
            )
            .with_subject([oxplow_domain::uri::build::config_uri(&key)]),
        );
        writes.push((key, value));
    }
    if writes.is_empty() {
        return Ok(HandlerOutput {
            result: json!({"changed":false}),
            unchanged: true,
            ..Default::default()
        });
    }
    let inverse = if inverses.len() == 1 {
        inverses.pop()
    } else {
        let mut set = serde_json::Map::new();
        let mut unset = Vec::new();
        for inverse in inverses {
            let key = inverse.input["key"]
                .as_str()
                .expect("setting key")
                .to_owned();
            if inverse.name == SET {
                set.insert(key, inverse.input["value"].clone());
            } else {
                unset.push(key);
            }
        }
        Some(CommandCall {
            name: "oxplow.config.update".into(),
            input: json!({"set":set,"unset":unset,"level":layer.level()}),
        })
    };
    // The file is an external system: complete its one atomic replacement
    // before the bus records success, events or undo.
    store.update(&writes, layer.level()).map_err(fail)?;
    target.refresh()?;
    Ok(HandlerOutput {
        result: json!({"changed":true}),
        inverse,
        events,
        ..Default::default()
    })
}

pub(super) fn settings_handler(
    action: impl Fn(&Actor, Value) -> Result<HandlerOutput, CommandError> + Send + Sync + 'static,
) -> Handler {
    let action = Arc::new(action);
    Handler::External(Arc::new(move |invocation, input| {
        let action = action.clone();
        Box::pin(async move { action(invocation.actor(), input) })
    }))
}

/// The four `config.*` commands over `target`.
pub fn ops(target: ConfigTarget) -> Vec<Op> {
    let target_for_settings = target.clone();
    let mark_generated = {
        let t = target.clone();
        Op::new(
            "config.write",
            "mark_generated",
            schema::<MarkGeneratedInput>(),
            true,
            settings_handler(move |actor, input| {
                let input: MarkGeneratedInput = parse(input)?;
                let entry = input.entry.trim().trim_matches('/').to_string();
                if entry.is_empty() {
                    return Err(CommandError::Invalid {
                        field: Some("/entry".into()),
                        message: "name a file or folder".into(),
                    });
                }
                let mut exclude: Vec<String> = Layer::Project
                    .value(&t.settings()?, "generated.exclude")?
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|error| CommandError::Invalid {
                        field: Some("/entry".into()),
                        message: error.to_string(),
                    })?
                    .unwrap_or_default();
                exclude.retain(|excluded| excluded != &entry);
                if input.generated {
                    exclude.push(entry);
                    exclude.sort();
                }
                change(
                    &t,
                    actor,
                    "generated.exclude",
                    Some(json!(exclude)),
                    Layer::Project,
                )
            }),
        )
    };
    let choose_provider = {
        let t = target.clone();
        Op::new(
            "config.write",
            "choose_provider",
            schema::<ChooseProviderInput>(),
            true,
            settings_handler(move |actor, input| {
                let input: ChooseProviderInput = parse(input)?;
                let capability = input.capability.trim().to_string();
                if capability.is_empty() {
                    return Err(CommandError::Invalid {
                        field: Some("/capability".into()),
                        message: "name a capability".into(),
                    });
                }
                if !oxplow_domain::capability::spec(&capability).is_some_and(|spec| spec.choosable)
                {
                    return Err(CommandError::Invalid {
                        field: Some("/capability".into()),
                        message: "name a choosable capability".into(),
                    });
                }
                let provider = input
                    .provider
                    .as_deref()
                    .map(str::trim)
                    .filter(|provider| !provider.is_empty());
                // Each capability is independent. Reset removes just this
                // scalar override and exposes its inherited choice.
                change(
                    &t,
                    actor,
                    &format!("activeProviders.{capability}"),
                    provider.map(|provider| json!(provider)),
                    input.level,
                )
            }),
        )
    };
    let choose_replacement = {
        let t = target;
        Op::new(
            "config.write",
            "choose_replacement",
            schema::<ChooseReplacementInput>(),
            true,
            settings_handler(move |actor, input| {
                let input: ChooseReplacementInput = parse(input)?;
                let target = input.target.trim().to_string();
                if target.is_empty() {
                    return Err(CommandError::Invalid {
                        field: Some("/target".into()),
                        message: "name the component".into(),
                    });
                }
                let mut chosen: serde_json::Map<String, Value> = Layer::Project
                    .value(&t.settings()?, "replacements")?
                    .and_then(|v| v.as_object().cloned())
                    .unwrap_or_default();
                match input
                    .whose
                    .as_deref()
                    .map(str::trim)
                    .filter(|w| !w.is_empty())
                {
                    Some(whose) => {
                        chosen.insert(target, Value::String(whose.to_string()));
                    }
                    None => {
                        chosen.remove(&target);
                    }
                }
                change(
                    &t,
                    actor,
                    "replacements",
                    Some(Value::Object(chosen)),
                    Layer::Project,
                )
            }),
        )
    };
    let mut ops = super::settings_commands::ops(target_for_settings);
    ops.extend([mark_generated, choose_provider, choose_replacement]);
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{agent_policy::AgentPolicy, commands::CommandBus, event_pump::EventPump};
    use oxplow_db::{Database, SqliteEventLogStore};

    fn setup() -> (tempfile::TempDir, ConfigTarget, Arc<CommandBus>) {
        let dir = tempfile::tempdir().unwrap();
        let target = ConfigTarget {
            global_dir: dir.path().join("global"),
            secrets: Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
            catalog: Some(Arc::new(
                crate::extension::extension_catalog::ExtensionCatalog::for_project(dir.path()),
            )),
            config: Arc::new(RwLock::new(
                oxplow_config::load_project_config_with_global(dir.path(), None).unwrap(),
            )),
            project_dir: dir.path().to_path_buf(),
            events: EventBus::new(),
            applied: Arc::new(tokio::sync::Notify::new()),
        };
        let db = Database::in_memory();
        let log = SqliteEventLogStore::new(
            db.clone(),
            oxplow_domain::vocabulary::VocabularyHandle::core(),
        );
        let pump = Arc::new(EventPump::new(db.clone(), log.clone(), vec![]));
        let bus = Arc::new(CommandBus::new(db, log, Arc::new(AgentPolicy), pump));
        for op in ops(target.clone()) {
            bus.add_op(op).unwrap();
        }
        crate::extension::extension_commands::register_declared(&bus);
        (dir, target, bus)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn settings_levels_override_and_unset_reveals_inheritance() {
        let (dir, target, bus) = setup();
        let key = "oxplow.snapshots.retentionDays";
        for (level, value) in [("global", 20), ("project", 10), ("local", 3)] {
            bus.run(
                &Actor::Human,
                SET,
                json!({"key":key,"value":value,"level":level}),
                false,
            )
            .await
            .unwrap();
        }
        assert_eq!(target.config.read().unwrap().snapshot_retention_days, 3);
        let read = bus
            .run(&Actor::Human, GET, json!({"key":key}), false)
            .await
            .unwrap();
        assert_eq!(read.result["value"], 3);
        assert_eq!(read.result["origin"], "local");
        bus.run(
            &Actor::Human,
            UNSET,
            json!({"key":key,"level":"local"}),
            false,
        )
        .await
        .unwrap();
        assert_eq!(target.config.read().unwrap().snapshot_retention_days, 10);
        let project = std::fs::read_to_string(dir.path().join(".oxplow/project.yaml")).unwrap();
        assert!(project.contains("oxplow.snapshots.retentionDays: 10"));
        assert!(dir.path().join("global/global.yaml").exists());
        assert!(!dir.path().join(".oxplow/personal.yaml").exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn settings_require_a_level_and_reject_invalid_values_before_storage() {
        let (dir, target, bus) = setup();
        for input in [
            json!({"key":"oxplow.snapshots.retentionDays","value":5}),
            json!({"key":"snapshotRetentionDays","value":5,"level":"project"}),
            json!({"key":"oxplow.snapshots.retentionDays","value":"soon","level":"project"}),
            json!({"key":"oxplow.agents","value":[],"level":"project"}),
        ] {
            assert!(bus.run(&Actor::Human, SET, input, false).await.is_err());
        }
        assert!(!dir.path().join(".oxplow/project.yaml").exists());
        assert_eq!(target.config.read().unwrap().snapshot_retention_days, 7);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn higher_overrides_cannot_hide_invalid_shared_writes() {
        let (_dir, target, bus) = setup();
        let key = "oxplow.replacements";
        let store = target.settings().unwrap();
        store
            .set(key, json!({"task.board":"oxplow"}), Level::Local)
            .unwrap();
        target.refresh().unwrap();
        for (level, label) in [(Level::Project, "project"), (Level::Global, "global")] {
            // This object passes the registry's string-map schema; the
            // typed capability rules must reject it before any file change.
            assert!(bus
                .run(
                    &Actor::Human,
                    SET,
                    json!({"key":key,"value":{"unknown.target":"invalid"},"level":label}),
                    false
                )
                .await
                .is_err());
            assert!(store.get(key, Some(level)).unwrap().value.is_none());
            // Convenience domain actions share the same preflight path.
            assert!(bus
                .run(
                    &Actor::Human,
                    "oxplow.replacement.choose",
                    json!({"target":"unknown.target","whose":"invalid"}),
                    false
                )
                .await
                .is_err());
            assert!(store.get(key, Some(level)).unwrap().value.is_none());
        }
        assert_eq!(
            store.get(key, Some(Level::Local)).unwrap().value,
            Some(json!({"task.board":"oxplow"}))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn settings_whole_maps_replace_and_canonical_undo_restores_the_layer() {
        let (_, target, bus) = setup();
        let key = "oxplow.replacements";
        bus.run(
            &Actor::Human,
            SET,
            json!({"key":key,"value":{"task.board":"oxplow"},"level":"global"}),
            false,
        )
        .await
        .unwrap();
        let out = bus
            .run(
                &Actor::Human,
                SET,
                json!({"key":key,"value":{},"level":"local"}),
                false,
            )
            .await
            .unwrap();
        assert!(target.config.read().unwrap().replacements.is_empty());
        let inverse = out.inverse.unwrap();
        assert_eq!(inverse.input["level"], "local");
        bus.run(&Actor::Human, &inverse.name, inverse.input, false)
            .await
            .unwrap();
        assert!(target
            .config
            .read()
            .unwrap()
            .replacements
            .contains_key("task.board"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn secret_settings_share_the_api_without_plaintext_or_audit_values() {
        let (dir, target, bus) = setup();
        bus.run(&Actor::Human, SET, json!({"key":"oxplow.ai.openai.baseUrl","value":"https://example.test/v1","level":"project"}), false).await.unwrap();
        let key = crate::ai_service::provider_key_setting("openai");
        let marker = "fixture-sensitive-token";
        let out = bus
            .run(
                &Actor::Human,
                SET,
                json!({"key":key,"value":marker,"level":"local"}),
                false,
            )
            .await
            .unwrap();
        assert!(out.inverse.is_none());
        let read = bus
            .run(&Actor::Human, GET, json!({"key":key}), false)
            .await
            .unwrap();
        assert_eq!(read.result["value"], marker);
        assert_eq!(read.result["origin"], "local");
        let inventory = target.settings().unwrap().inventory().unwrap();
        let secret = inventory
            .iter()
            .find(|entry| entry.definition.key == key)
            .unwrap();
        assert!(secret.is_set && secret.value.is_none());
        for path in [
            dir.path().join(".oxplow/project.yaml"),
            dir.path().join(".oxplow/local.yaml"),
            dir.path().join("global/global.yaml"),
        ] {
            assert!(!std::fs::read_to_string(path)
                .unwrap_or_default()
                .contains(marker));
        }
        let audits = bus.db.read(|conn| {
            let mut stmt = conn.prepare("SELECT coalesce(input_json,'') || coalesce(result_json,'') || coalesce(inverse_json,'') FROM command_audit").map_err(oxplow_db::map_sql_err)?;
            let rows = stmt.query_map([], |row| row.get::<_,String>(0)).map_err(oxplow_db::map_sql_err)?;
            rows.collect::<Result<Vec<_>,_>>().map_err(oxplow_db::map_sql_err)
        }).await.unwrap();
        assert!(audits.iter().all(|row| !row.contains(marker)));
        // Changing a destination cannot carry an existing token to that host.
        bus.run(&Actor::Human, SET, json!({"key":"oxplow.ai.openai.baseUrl","value":"https://other.test/v1","level":"project"}), false).await.unwrap();
        assert!(bus
            .run(&Actor::Human, GET, json!({"key":key}), false)
            .await
            .is_err());
        bus.run(
            &Actor::Human,
            SET,
            json!({"key":key,"value":"rotated-fixture-token","level":"local"}),
            false,
        )
        .await
        .unwrap();
        let rotated = bus
            .run(&Actor::Human, GET, json!({"key":key}), false)
            .await
            .unwrap();
        assert_eq!(rotated.result["value"], "rotated-fixture-token");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn project_map_actions_do_not_copy_local_overrides() {
        let (_dir, target, bus) = setup();
        let store = target.settings().unwrap();
        let key = "oxplow.replacements";
        for (level, value) in [
            (Level::Global, json!({"task.board":"global_view"})),
            (Level::Project, json!({"task.list":"project_view"})),
            (Level::Local, json!({"task.detail":"local_view"})),
        ] {
            store.set(key, value, level).unwrap();
        }
        target.refresh().unwrap();
        assert_eq!(
            Layer::Global.value(&store, key).unwrap(),
            Some(json!({"task.board":"global_view"}))
        );
        assert_eq!(
            Layer::Project.value(&store, key).unwrap(),
            Some(json!({"task.list":"project_view"}))
        );
        assert_eq!(
            Layer::Local.value(&store, key).unwrap(),
            Some(json!({"task.detail":"local_view"}))
        );
        bus.run(
            &Actor::Human,
            "oxplow.replacement.choose",
            json!({"target":"task.board","whose":"project_new"}),
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            store.get(key, Some(Level::Project)).unwrap().value,
            Some(json!({"task.list":"project_view","task.board":"project_new"}))
        );
        assert_eq!(
            store.get(key, Some(Level::Local)).unwrap().value,
            Some(json!({"task.detail":"local_view"}))
        );
        // Clearing an inherited final entry masks it with an empty map.
        store.unset(key, Level::Project).unwrap();
        bus.run(
            &Actor::Human,
            "oxplow.replacement.choose",
            json!({"target":"task.board"}),
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            store.get(key, Some(Level::Project)).unwrap().value,
            Some(json!({}))
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn generated_actions_change_only_the_project_exclude_leaf() {
        let (_dir, target, bus) = setup();
        let store = target.settings().unwrap();
        for (level, include) in [
            (Level::Global, "global/**"),
            (Level::Project, "project/**"),
            (Level::Local, "local/**"),
        ] {
            store
                .set("oxplow.generated.include", json!([include]), level)
                .unwrap();
        }
        store
            .set(
                "oxplow.generated.exclude",
                json!(["inherited"]),
                Level::Global,
            )
            .unwrap();
        store
            .set("oxplow.generated.exclude", json!(["private"]), Level::Local)
            .unwrap();
        target.refresh().unwrap();
        bus.run(
            &Actor::Human,
            "oxplow.file.mark_generated",
            json!({"entry":"dist"}),
            false,
        )
        .await
        .unwrap();
        assert_eq!(
            store
                .get("oxplow.generated.exclude", Some(Level::Project))
                .unwrap()
                .value,
            Some(json!(["dist", "inherited"]))
        );
        for (level, include) in [
            (Level::Global, "global/**"),
            (Level::Project, "project/**"),
            (Level::Local, "local/**"),
        ] {
            assert_eq!(
                store
                    .get("oxplow.generated.include", Some(level))
                    .unwrap()
                    .value,
                Some(json!([include]))
            );
        }
        for entry in ["dist", "inherited"] {
            bus.run(
                &Actor::Human,
                "oxplow.file.mark_generated",
                json!({"entry":entry,"generated":false}),
                false,
            )
            .await
            .unwrap();
        }
        assert_eq!(
            store
                .get("oxplow.generated.exclude", Some(Level::Project))
                .unwrap()
                .value,
            Some(json!([]))
        );
        assert_eq!(
            store
                .get("oxplow.generated.exclude", Some(Level::Local))
                .unwrap()
                .value,
            Some(json!(["private"]))
        );
    }

    #[tokio::test]
    async fn domain_settings_complete_external_writes_before_reporting_success() {
        let (dir, target, _) = setup();
        let run =
            oxplow_domain::run::RunContext::core(Actor::Human, oxplow_domain::run::Mode::External);
        let op = ops(target)
            .into_iter()
            .find(|op| op.name == "mark_generated")
            .unwrap();
        let Handler::External(handler) = op.handler else {
            panic!("settings writes must run outside the database transaction")
        };
        let invocation = super::super::Invocation {
            run,
            idempotency_key: None,
            confirmed: true,
        };
        let first = handler(invocation.clone(), json!({"entry":"dist"}))
            .await
            .unwrap();
        assert!(first.after_commit.is_none());
        assert!(first.inverse.is_some());
        let path = dir.path().join(".oxplow/project.yaml");
        let before = std::fs::read_to_string(&path).unwrap();
        assert!(before.contains("oxplow.generated.exclude:"));
        let second = handler(invocation, json!({"entry":"dist"})).await.unwrap();
        assert_eq!(second.result["changed"], false);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    }
}
