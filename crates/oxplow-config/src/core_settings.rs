//! Core's declarations in the shared settings registry, and the typed
//! runtime view built from them.
//!
//! [`table`] declares each fixed core setting once: its registered key,
//! metadata, schema, default and how its value applies to [`OxplowConfig`].
//! A few key families (a capability's choice, a harness's switch and
//! preferences, an AI role) have one setting per member and are applied by
//! [`config_from`] by their key's shape. Values are atomic: objects and
//! arrays replace whole values, nothing merges between levels.

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::settings::{Level, Origin, SettingDefinition, SettingsError};
use crate::{ConfigError, OxplowConfig};

/// The registered keys core code names directly.
pub mod keys {
    pub const PROJECT_NAME: &str = "oxplow.projectName";
    pub const AGENT_PROMPT_APPEND: &str = "oxplow.agentPromptAppend";
    pub const INJECT_SESSION_CONTEXT: &str = "oxplow.injectSessionContext";
    pub const ICON_TINT: &str = "oxplow.iconTint";
    pub const SNAPSHOT_RETENTION_DAYS: &str = "oxplow.snapshots.retentionDays";
    pub const SNAPSHOT_MAX_FILE_BYTES: &str = "oxplow.snapshots.maxFileBytes";
    pub const METRICS: &str = "oxplow.metrics";
    pub const METRIC_RETENTION_DAYS: &str = "oxplow.metrics.retentionDays";
    pub const METRIC_DETAIL_MAX_PER_PRODUCER: &str = "oxplow.metrics.detailMaxPerProducer";
    pub const METRIC_DETAIL_RETENTION_DAYS: &str = "oxplow.metrics.detailRetentionDays";
    pub const GENERATED_EXCLUDE: &str = "oxplow.generated.exclude";
    pub const GENERATED_INCLUDE: &str = "oxplow.generated.include";
    pub const FILES_KINDS: &str = "oxplow.files.kinds";
    pub const TESTING_COMMAND: &str = "oxplow.testing.command";
    pub const TESTING_FAST_COMMAND: &str = "oxplow.testing.fastCommand";
    pub const TESTING_RUN_PATTERNS: &str = "oxplow.testing.runPatterns";
    pub const TESTING_ANALYSIS_PATTERNS: &str = "oxplow.testing.analysisPatterns";
    pub const TESTING_TEST_PATHS: &str = "oxplow.testing.testPaths";
    pub const TESTING_AGENT_HINT: &str = "oxplow.testing.agentHint";
    pub const SCRIPTS_MAX_CONCURRENT: &str = "oxplow.scripts.maxConcurrent";
    pub const SCRIPTS_MEMORY_MB: &str = "oxplow.scripts.memoryMb";
    pub const LSP_SERVERS: &str = "oxplow.lsp.servers";
    pub const EXTENSION_INSTANCES: &str = "oxplow.extensionInstances";
    pub const REPLACEMENTS: &str = "oxplow.replacements";
    pub const EVENT_RETENTION: &str = "oxplow.eventRetention";
    pub const EXTENSION_SOURCES: &str = "oxplow.extensionSources";
    /// `oxplow.activeProviders.<capability>`: [`super::active_provider_key`].
    pub const ACTIVE_PROVIDERS_PREFIX: &str = "oxplow.activeProviders.";
    /// `oxplow.agents.<harness>.enabled` and `.<preference>`.
    pub const AGENTS_PREFIX: &str = "oxplow.agents.";
}

type Apply = fn(&mut OxplowConfig, Value) -> Result<(), String>;

/// One fixed core setting: its registered definition and how its effective
/// value (or its default) applies to the typed runtime view.
pub struct CoreSetting {
    pub definition: SettingDefinition,
    apply: Apply,
}

fn setting(
    key: &str,
    title: &str,
    category: &str,
    description: &str,
    schema: Value,
    default: Option<Value>,
    apply: Apply,
) -> CoreSetting {
    CoreSetting {
        definition: SettingDefinition {
            key: key.into(),
            title: title.into(),
            description: description.into(),
            category: category.into(),
            schema,
            default,
            secret: false,
            order: 0,
            ui: Default::default(),
            mask: Default::default(),
            owner: "core".into(),
        },
        apply,
    }
}

impl CoreSetting {
    fn multiline(mut self) -> Self {
        self.definition.ui = crate::settings::SettingUi {
            editor: Some(crate::settings::SettingEditor::Multiline),
        };
        self
    }
}

/// A whole value's JSON Schema from its Rust type. Nested types stay in the
/// root's own definitions, so their references resolve.
fn schema_of<T: schemars::JsonSchema>() -> Value {
    let mut schema = serde_json::to_value(schemars::schema_for!(T)).expect("schema serializes");
    if let Some(object) = schema.as_object_mut() {
        object.remove("$schema");
        object.remove("title");
    }
    schema
}

fn de<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| error.to_string())
}

fn whole(value: Value) -> Result<u64, String> {
    // YAML can write a whole number as `7.0`; JSON Schema's `integer` takes it.
    value
        .as_u64()
        .or_else(|| {
            value
                .as_f64()
                .filter(|n| n.fract() == 0.0 && *n >= 0.0 && *n <= u64::MAX as f64)
                .map(|n| n as u64)
        })
        .ok_or_else(|| "must be a whole number".to_owned())
}

fn u32_of(value: Value) -> Result<u32, String> {
    u32::try_from(whole(value)?).map_err(|_| "is too large".to_owned())
}

fn text(value: Value) -> Result<Option<String>, String> {
    Ok(de::<Option<String>>(value)?
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty()))
}

fn string_list(value: Value) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for (index, item) in de::<Option<Vec<String>>>(value)?
        .into_iter()
        .flatten()
        .enumerate()
    {
        let trimmed = item.trim().to_owned();
        if trimmed.is_empty() {
            return Err(format!("item {} must be a non-empty string", index + 1));
        }
        out.push(trimmed);
    }
    Ok(out)
}

fn glob_list(value: Value) -> Result<Vec<String>, String> {
    let list = string_list(value)?;
    for (index, glob) in list.iter().enumerate() {
        globset::GlobBuilder::new(glob)
            .literal_separator(true)
            .build()
            .map_err(|e| format!("item {} `{glob}` isn't a valid glob: {e}", index + 1))?;
    }
    Ok(list)
}

/// `oxplow.files.kinds`: each rule's kind is lowercase-dashed and its glob
/// parses (as `testPaths` do).
fn file_kind_rules(value: Value) -> Result<Vec<crate::FileKindRule>, String> {
    let rules = de::<Option<Vec<crate::FileKindRule>>>(value)?.unwrap_or_default();
    for (index, rule) in rules.iter().enumerate() {
        let item = index + 1;
        if !valid_file_kind(&rule.kind) {
            return Err(format!(
                "item {item} kind `{}` must be lowercase letters, digits and dashes",
                rule.kind
            ));
        }
        globset::GlobBuilder::new(&rule.glob)
            .literal_separator(true)
            .build()
            .map_err(|e| format!("item {item} `{}` isn't a valid glob: {e}", rule.glob))?;
    }
    Ok(rules)
}

/// Whether `kind` is a file kind's name: lowercase letters and digits in
/// dash-separated words (`lint-config`).
pub fn valid_file_kind(kind: &str) -> bool {
    !kind.is_empty()
        && kind.split('-').all(|word| {
            !word.is_empty()
                && word
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        })
}

fn generated(value: Value, key: &str) -> Result<Vec<String>, String> {
    crate::validate_generated_list(de::<Vec<String>>(value)?, key).map_err(message)
}

fn message(error: ConfigError) -> String {
    match error {
        ConfigError::Invalid(message) => message,
        other => other.to_string(),
    }
}

/// Core's Settings categories, in the order the Settings index lists them.
pub fn categories() -> Vec<crate::settings::SettingCategory> {
    [
        ("project", "Project", "The project's name and icon colour."),
        ("agents", "Agents", "Which agent harnesses run, how each one is set up, and what every agent is told."),
        ("ai", "AI", "AI providers, and which model each role uses."),
        ("files", "Files", "Which files are generated and left out of snapshots and changes."),
        ("snapshots", "Snapshots", "How long snapshots are kept and which files they hold."),
        ("testing", "Testing", "How tests run and which files are tests."),
        ("metrics", "Metrics", "Which metrics are on and how long their facts are kept."),
        ("scripts", "Scripts", "Limits for extension scripts."),
        ("languageServers", "Language servers", "The language servers this project runs."),
        ("integrations", "Integrations", "Which implementation each capability uses, provider instances and component replacements."),
        ("data", "Data", "How long events are kept."),
        ("extensions", "Extensions", "Which extensions are on, and where they come from."),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (id, title, description))| crate::settings::SettingCategory {
        id: id.into(),
        title: title.into(),
        description: description.into(),
        order: i32::try_from(index).expect("few categories") * 10,
        owner: "core".into(),
    })
    .collect()
}

/// Every fixed core setting, with defaults evaluated for this project's name.
pub fn table(fallback_name: &str) -> Vec<CoreSetting> {
    use crate::*;
    let fallback_name = if fallback_name.trim().is_empty() {
        "Project"
    } else {
        fallback_name
    };
    let nullable_text = || json!({"type": ["string", "null"]});
    let nullable_list = || json!({"type": ["array", "null"], "items": {"type": "string"}});
    let count = |minimum: u64| json!({"type": "integer", "minimum": minimum});
    vec![
        setting(
            keys::PROJECT_NAME,
            "Project name",
            "project",
            "Display name; defaults to the project directory's basename.",
            json!({"type": "string", "minLength": 1, "pattern": "\\S"}),
            Some(json!(fallback_name)),
            |c, v| {
                c.project_name = text(v)?.ok_or("must be a non-empty name")?;
                Ok(())
            },
        ),
        setting(
            keys::ICON_TINT,
            "Icon tint",
            "project",
            "Hex colour composited behind the app icon so windows are tellable apart (macOS).",
            json!({"type": ["string", "null"], "pattern": "^#?([a-fA-F0-9]{3}|[a-fA-F0-9]{6})$"}),
            None,
            |c, v| {
                c.icon_tint = de::<Option<String>>(v)?;
                if let Some(tint) = c.icon_tint.as_deref().filter(|t| parse_hex_rgb(t).is_none())
                {
                    return Err(format!(
                        "must be a hex colour like \"#c2410c\" or \"#abc\"; got {tint:?}"
                    ));
                }
                Ok(())
            },
        ),
        setting(
            keys::AGENT_PROMPT_APPEND,
            "Agent prompt append",
            "agents",
            "Text appended verbatim to every agent's system prompt. Steers every agent.",
            nullable_text(),
            Some(json!("")),
            |c, v| {
                c.agent_prompt_append = de::<Option<String>>(v)?.unwrap_or_default();
                Ok(())
            },
        )
        .multiline(),
        setting(
            keys::INJECT_SESSION_CONTEXT,
            "Inject session context",
            "agents",
            "Inject the session-context block into every agent prompt.",
            json!({"type": ["boolean", "null"]}),
            Some(json!(DEFAULT_INJECT_SESSION_CONTEXT)),
            |c, v| {
                c.inject_session_context =
                    de::<Option<bool>>(v)?.unwrap_or(DEFAULT_INJECT_SESSION_CONTEXT);
                Ok(())
            },
        ),
        setting(
            keys::SNAPSHOT_RETENTION_DAYS,
            "Snapshot retention days",
            "snapshots",
            "File-snapshot retention in days; 0 disables pruning.",
            count(0),
            Some(json!(DEFAULT_SNAPSHOT_RETENTION_DAYS)),
            |c, v| {
                c.snapshot_retention_days = u32_of(v)?;
                Ok(())
            },
        ),
        setting(
            keys::SNAPSHOT_MAX_FILE_BYTES,
            "Snapshot max file bytes",
            "snapshots",
            "Largest file snapshotted by content; bigger files get a stat-only entry.",
            count(1024),
            Some(json!(DEFAULT_SNAPSHOT_MAX_FILE_BYTES)),
            |c, v| {
                c.snapshot_max_file_bytes = whole(v)?;
                Ok(())
            },
        ),
        setting(
            keys::METRICS,
            "Metrics",
            "metrics",
            "Which metrics are on: `use` turns one on (with optional target/warnAt/failAt), `enabled: false` turns an on-by-default one off. Metrics are defined in extensions.",
            json!({"type": "array", "items": {"type": "object", "additionalProperties": false,
                "required": ["use"], "properties": {
                    "use": {"type": "string", "minLength": 1, "title": "Metric"},
                    "enabled": {"type": ["boolean", "null"]},
                    "target": {"type": ["number", "null"]},
                    "warnAt": {"type": ["number", "null"]},
                    "failAt": {"type": ["number", "null"]}
                }}}),
            Some(json!([])),
            |c, v| {
                c.metrics = validate_metrics(Some(de(v)?), CatalogLayer::Project).map_err(message)?;
                Ok(())
            },
        ),
        setting(
            keys::METRIC_RETENTION_DAYS,
            "Metric retention days",
            "metrics",
            "Metric-capture retention in days; 0 (default) keeps everything.",
            count(0),
            Some(json!(DEFAULT_METRIC_RETENTION_DAYS)),
            |c, v| {
                c.metric_retention_days = u32_of(v)?;
                Ok(())
            },
        ),
        setting(
            keys::METRIC_DETAIL_MAX_PER_PRODUCER,
            "Metric detail max per producer",
            "metrics",
            "Keep per-run detail for only the newest N captures per producer; 0 disables the cap.",
            count(0),
            Some(json!(DEFAULT_METRIC_DETAIL_MAX_PER_PRODUCER)),
            |c, v| {
                c.metric_detail_max_per_producer = u32_of(v)?;
                Ok(())
            },
        ),
        setting(
            keys::METRIC_DETAIL_RETENTION_DAYS,
            "Metric detail retention days",
            "metrics",
            "Compact per-run detail older than this many days; 0 disables.",
            count(0),
            Some(json!(DEFAULT_METRIC_DETAIL_RETENTION_DAYS)),
            |c, v| {
                c.metric_detail_retention_days = u32_of(v)?;
                Ok(())
            },
        ),
        setting(
            keys::GENERATED_EXCLUDE,
            "Generated: exclude",
            "files",
            "Paths oxplow ignores even when `.gitignore` doesn't (watching, snapshots and scans): a single segment (`target`) matches any path component; a repo-relative path (`apps/desktop/dist`) matches that path and what is under it.",
            json!({"type": "array", "items": {"type": "string"}}),
            Some(json!([])),
            |c, v| {
                c.generated.exclude = generated(v, keys::GENERATED_EXCLUDE)?;
                Ok(())
            },
        ),
        setting(
            keys::GENERATED_INCLUDE,
            "Generated: include",
            "files",
            "Paths `.gitignore` ignores that oxplow still watches, snapshots and scans, written as for the exclude list.",
            json!({"type": "array", "items": {"type": "string"}}),
            Some(json!([])),
            |c, v| {
                c.generated.include = generated(v, keys::GENERATED_INCLUDE)?;
                Ok(())
            },
        ),
        setting(
            keys::TESTING_COMMAND,
            "Test command",
            "testing",
            "The command that runs the project's whole test suite.",
            nullable_text(),
            None,
            |c, v| {
                c.testing.command = text(v)?;
                Ok(())
            },
        ),
        setting(
            keys::TESTING_FAST_COMMAND,
            "Fast test command",
            "testing",
            "A quicker test command for the inner loop, when the project has one.",
            nullable_text(),
            None,
            |c, v| {
                c.testing.fast_command = text(v)?;
                Ok(())
            },
        ),
        setting(
            keys::TESTING_RUN_PATTERNS,
            "Test run patterns",
            "testing",
            "Extra command substrings that count as a test run, on top of the ones the enabled extensions declare (`run_patterns:` of kind `test`).",
            nullable_list(),
            Some(json!([])),
            |c, v| {
                c.testing.run_patterns = string_list(v)?;
                Ok(())
            },
        ),
        setting(
            keys::TESTING_ANALYSIS_PATTERNS,
            "Analysis run patterns",
            "testing",
            "Extra command substrings that count as a static-analysis run, on top of the ones the enabled extensions declare (`run_patterns:` of kind `analysis`).",
            nullable_list(),
            Some(json!([])),
            |c, v| {
                c.testing.analysis_patterns = string_list(v)?;
                Ok(())
            },
        ),
        setting(
            keys::FILES_KINDS,
            "File kinds",
            "project",
            "The project's own file-kind rules, on top of what the enabled extensions declare (`file_kinds:`): `[{ kind: generated, glob: \"gen/**\" }]`. A kind is lowercase letters, digits and dashes (`ci`, `sensitive`, `lockfile`, …).",
            json!({"type": ["array", "null"], "items": schema_of::<crate::FileKindRule>()}),
            Some(json!([])),
            |c, v| {
                c.files.kinds = file_kind_rules(v)?;
                Ok(())
            },
        ),
        setting(
            keys::TESTING_TEST_PATHS,
            "Test paths",
            "testing",
            "Globs (repo-relative) naming test files, on top of the built-in conventions (`.test.` and `.spec.` files, `tests/` folders, …): `spec/**`.",
            nullable_list(),
            Some(json!([])),
            |c, v| {
                c.testing.test_paths = glob_list(v)?;
                Ok(())
            },
        ),
        setting(
            keys::TESTING_AGENT_HINT,
            "Testing hint for agents",
            "testing",
            "Text added to every agent's system prompt about testing: which command to run, what coverage to meet.",
            nullable_text(),
            None,
            |c, v| {
                c.testing.agent_hint = text(v)?;
                Ok(())
            },
        )
        .multiline(),
        setting(
            keys::SCRIPTS_MAX_CONCURRENT,
            "Script workers at once",
            "scripts",
            "How many script workers run at once, across the host; a run past it waits for a slot. Default: 4, or fewer on a machine with fewer cores.",
            count(1),
            Some(json!(ScriptsConfig::default().max_concurrent)),
            |c, v| {
                c.scripts.max_concurrent = usize::try_from(whole(v)?).map_err(|e| e.to_string())?;
                Ok(())
            },
        ),
        setting(
            keys::SCRIPTS_MEMORY_MB,
            "Script worker memory (MiB)",
            "scripts",
            "The most memory one script worker may use before it is stopped, in MiB.",
            count(MIN_SCRIPTS_MEMORY_MB),
            Some(json!(DEFAULT_SCRIPTS_MEMORY_MB)),
            |c, v| {
                c.scripts.memory_bytes = whole(v)? * 1024 * 1024;
                Ok(())
            },
        ),
        setting(
            keys::LSP_SERVERS,
            "Language servers",
            "languageServers",
            "Extra language servers, each `{ languageId, extensions, command, args }`. Runs programs.",
            schema_of::<Option<Vec<RawLspServer>>>(),
            Some(json!([])),
            |c, v| {
                c.lsp_servers = validate_lsp_servers(de::<Option<Vec<RawLspServer>>>(v)?.unwrap_or_default())
                    .map_err(message)?;
                Ok(())
            },
        ),
        setting(
            keys::EXTENSION_INSTANCES,
            "Extension instances",
            "integrations",
            "Instances of extension providers, by `<extension>/<instance id>`: `{ enabled, config, provider? }` (the provider's instance config; `provider` names which of the extension's providers an instance with its own id is). Enabling one runs its program once this machine approved it.",
            schema_of::<Option<BTreeMap<String, ExtensionInstanceConfig>>>(),
            Some(json!({})),
            |c, v| {
                c.extension_instances =
                    validate_extension_instances(de::<Option<_>>(v)?.unwrap_or_default())
                        .map_err(message)?;
                Ok(())
            },
        ),
        setting(
            keys::REPLACEMENTS,
            "Replacements",
            "integrations",
            "Whose component shows for a replaceable core sub-component, by target: `{ task.board: task-views, task.list: oxplow }` — an extension replacing it, or `oxplow` for oxplow's own. Unlisted, the only extension replacing it shows (several: oxplow's own); a bound target (`task.detail.state`) shows its provider's extension's, and may only be set to `oxplow`.",
            json!({"type": ["object", "null"], "additionalProperties": {"type": "string"}}),
            Some(json!({})),
            |c, v| {
                c.replacements =
                    validate_replacements(de::<Option<_>>(v)?.unwrap_or_default()).map_err(message)?;
                Ok(())
            },
        ),
        setting(
            keys::EVENT_RETENTION,
            "Event retention",
            "data",
            "How long each event namespace's payloads and large content are kept, in days: `{ agent: { payloadDays: 60, contentDays: 14 } }`, over core's defaults (agent 30/14; test, code, collector, effect 90/30). An extension's namespace is kept no longer than its window; core state (snapshot, vcs, effort, …) is kept whole.",
            schema_of::<Option<BTreeMap<String, oxplow_domain::events::retention::RetentionWindow>>>(),
            Some(json!({})),
            |c, v| {
                c.event_retention =
                    validate_event_retention(de::<Option<_>>(v)?.unwrap_or_default())
                        .map_err(message)?;
                Ok(())
            },
        ),
        setting(
            keys::EXTENSION_SOURCES,
            "Extension sources",
            "extensions",
            "Where an extension's source is when it isn't `.oxplow/extensions/<name>/`: `{ <name>: <folder> }`, relative to the checkout or absolute (`~/` is the home folder). Enables code.",
            json!({"type": ["object", "null"], "additionalProperties": {"type": "string"}}),
            Some(json!({})),
            |c, v| {
                c.extension_sources = de::<Option<_>>(v)?.unwrap_or_default();
                Ok(())
            },
        ),
    ]
}

/// All core definitions: the fixed table and each family's current members.
pub fn definitions(fallback_name: &str) -> Vec<SettingDefinition> {
    let mut out: Vec<_> = table(fallback_name)
        .into_iter()
        .map(|setting| setting.definition)
        .collect();
    out.extend(
        oxplow_domain::capability::choosable().map(|capability| SettingDefinition {
            key: active_provider_key(capability.id),
            title: format!("{} implementation", capability.title),
            description: format!(
                "The implementation used for {}. Unset to inherit the choice.",
                capability.title.to_lowercase()
            ),
            category: "integrations".into(),
            schema: json!({"type": "string", "pattern": "^[a-z][a-z0-9_]*$"}),
            default: None,
            secret: false,
            order: 0,
            ui: Default::default(),
            mask: Default::default(),
            owner: "core".into(),
        }),
    );
    out.extend(
        crate::AI_ROLE_NAMES
            .iter()
            .map(|role| role_definition(role)),
    );
    out.extend(
        crate::DEFAULT_EXTENSIONS
            .iter()
            .map(|name| extension_definition(name)),
    );
    out.sort_by(|a, b| a.key.cmp(&b.key));
    out
}

/// The setting that chooses a capability's implementation.
pub fn active_provider_key(capability: &str) -> String {
    format!("{}{capability}", keys::ACTIVE_PROVIDERS_PREFIX)
}

fn core_definition(
    key: String,
    title: String,
    description: &str,
    category: &str,
    schema: Value,
    default: Option<Value>,
) -> SettingDefinition {
    SettingDefinition {
        key,
        title,
        description: description.into(),
        category: category.into(),
        schema,
        default,
        secret: false,
        order: 0,
        ui: Default::default(),
        mask: Default::default(),
        owner: "core".into(),
    }
}

/// One independently inherited feature switch. Enabling does not deploy code.
pub fn extension_definition(name: &str) -> SettingDefinition {
    core_definition(
        format!("oxplow.extensions.{name}.enabled"),
        format!("Enable {name}"),
        "Whether this extension is enabled. Deployment and permission approval are separate.",
        "extensions",
        json!({"type": "boolean"}),
        Some(json!(true)),
    )
}

pub use oxplow_domain::ai::{role_from_setting_key as role_from_key, role_setting_key as role_key};

pub fn role_definition(role: &str) -> SettingDefinition {
    core_definition(
        role_key(role),
        format!("AI role {role}"),
        "Provider and model used for this role. Remove the override to inherit.",
        "ai",
        json!({"type": "object", "additionalProperties": false,
        "required": ["provider", "model"], "properties": {
            "provider": {"type": "string", "minLength": 1},
            "model": {"type": "string", "minLength": 1},
            "restrict": {"type": "array", "uniqueItems": true,
                "items": {"type": "string", "enum": ["tools", "sql", "streaming", "images"]}}
        }}),
        None,
    )
}

/// A clean break reports obsolete core keys rather than silently ignoring a
/// preference that could otherwise re-enable a feature.
pub fn retired_setting(key: &str) -> Option<&'static str> {
    match key {
        "oxplow.agents" | "oxplow.agentConfig" => {
            Some("use oxplow.agents.<harness>.enabled and the harness's declared preference keys")
        }
        "oxplow.extensions" | "oxplow.enableExtensions" | "oxplow.disableExtensions" => {
            Some("use oxplow.extensions.<name>.enabled")
        }
        "oxplow.ai.providers" => {
            Some("use oxplow.ai.<provider>.enabled, .baseUrl and secret .apiKey")
        }
        "oxplow.ai.roles" => Some("use one oxplow.ai.roles.<role> assignment per role"),
        "oxplow.ai.roles.main" => Some(
            "use oxplow.ai.roles.default: the main role is now default, the model every role without one uses",
        ),
        "oxplow.ai.roles.assistant" => {
            Some("oxplow has no Assistant now: remove it (roles: default, summarize, decide)")
        }
        "oxplow.zones" => Some(
            "oxplow has no zones now: a project that wants its own layers writes them as a dataset in its own extension",
        ),
        "oxplow.newThreadSession" => {
            Some("new threads open the picker; its last choice is remembered in UI state")
        }
        "oxplow.enableBundledExtensions" => Some(
            "a development build runs oxplow's own extensions from the sources the project names in oxplow.extensionSources",
        ),
        "oxplow.snapshotTurnBudgetMs" => Some("the hook wait timeout is now managed internally"),
        _ => None,
    }
}

/// Dynamic feature names can be discovered before extension activation.
pub fn register_dynamic_keys(
    registry: &mut crate::settings::SettingsRegistry,
    keys: impl IntoIterator<Item = String>,
) -> Result<(), SettingsError> {
    for key in keys {
        if let Some(name) = key
            .strip_prefix("oxplow.extensions.")
            .and_then(|s| s.strip_suffix(".enabled"))
        {
            if !name.is_empty() && registry.get(&key).is_none() {
                registry.register(extension_definition(name))?;
            }
        }
    }
    Ok(())
}

/// Internal metric entries share a type with extension definitions; persisted
/// preferences contain only the fields users can configure.
pub fn metric_preferences(value: &Value) -> Value {
    Value::Array(
        value
            .as_array()
            .into_iter()
            .flatten()
            .map(|entry| {
                Value::Object(
                    entry
                        .as_object()
                        .into_iter()
                        .flatten()
                        .filter(|(key, value)| {
                            matches!(
                                key.as_str(),
                                "use" | "enabled" | "target" | "warnAt" | "failAt"
                            ) && !value.is_null()
                        })
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect(),
                )
            })
            .collect(),
    )
}

/// The typed runtime view of resolved settings: every fixed setting's value
/// (else its default), then each family member that has a value. Keys core
/// does not own (extension settings) are ignored.
pub fn config_from(
    fallback_name: &str,
    values: &BTreeMap<String, (Value, Level)>,
) -> Result<OxplowConfig, ConfigError> {
    let invalid = |key: &str, message: String| ConfigError::Invalid(format!("{key}: {message}"));
    let mut config = crate::blank_config(fallback_name);
    for setting in table(fallback_name) {
        let key = &setting.definition.key;
        let value = values
            .get(key)
            .map(|(value, _)| value.clone())
            .or_else(|| setting.definition.default.clone())
            .unwrap_or(Value::Null);
        (setting.apply)(&mut config, value).map_err(|message| invalid(key, message))?;
    }
    for (key, (value, origin)) in values {
        apply_family(&mut config, key, value.clone(), *origin)
            .map_err(|message| invalid(key, message))?;
    }
    Ok(config)
}

/// Check one value by its own core rule, the rule `config_from` applies.
/// No rule reads another setting, so a write checks only the keys it
/// changes. A key that core does not own has no core rule.
pub fn check(key: &str, value: &Value) -> Result<(), ConfigError> {
    let invalid = |message: String| ConfigError::Invalid(format!("{key}: {message}"));
    let mut config = crate::blank_config("Project");
    match table("Project")
        .into_iter()
        .find(|setting| setting.definition.key == key)
    {
        Some(setting) => (setting.apply)(&mut config, value.clone()).map_err(invalid),
        None => apply_family(&mut config, key, value.clone(), Level::Project).map_err(invalid),
    }
}

fn apply_family(
    config: &mut OxplowConfig,
    key: &str,
    value: Value,
    origin: Level,
) -> Result<(), String> {
    if let Some(capability) = key.strip_prefix(keys::ACTIVE_PROVIDERS_PREFIX) {
        let provider: String = de(value)?;
        crate::validate_active_provider(capability, &provider).map_err(message)?;
        config
            .active_providers
            .insert(capability.to_owned(), provider);
        config
            .active_provider_origins
            .insert(capability.to_owned(), origin);
    } else if let Some(role) = role_from_key(key) {
        // Checked here; `AiService` reads the binding from the settings.
        let assignment: crate::AiRoleOverride = de(value)?;
        crate::validate_ai_role(&role, &assignment).map_err(message)?;
    } else if let Some(rest) = key.strip_prefix(keys::AGENTS_PREFIX) {
        let Some((harness, field)) = rest.rsplit_once('.') else {
            return Ok(());
        };
        let harness = harness.replace('.', "/");
        crate::validate_agent_key(&harness).map_err(message)?;
        if field == "enabled" {
            config.agent_enabled.insert(harness, de(value)?);
        } else {
            // A harness's preferences are its own to check.
            let field = crate::settings::decoded_segment(field).unwrap_or_else(|| field.to_owned());
            config
                .agent_config
                .entry(harness)
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .expect("a harness's preferences are an object")
                .insert(field, value);
        }
    }
    Ok(())
}

/// A resolved core value that failed, naming the setting key.
#[derive(Debug)]
pub struct CoreProjectionError {
    key: String,
    source: SettingsError,
}

impl std::fmt::Display for CoreProjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.key, self.source)
    }
}

impl std::error::Error for CoreProjectionError {}

/// The effective value and origin of every core setting that has one.
///
/// Registered keys resolve through the store (their schema checked). At
/// boot, a harness's preferences and an extension's AI role can be in a
/// level before their declaration is loaded: such a family key is carried
/// from the first level that sets it, an unknown role checked against the
/// role schema, an unknown preference kept as plain JSON for its harness.
/// A shadowed override never fails the load.
pub fn effective_values(
    snapshot: &crate::settings::SettingsSnapshot,
) -> Result<BTreeMap<String, (Value, Level)>, CoreProjectionError> {
    let store = snapshot.store();
    let definitions: Vec<_> = store
        .registry
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .definitions()
        .filter(|definition| !definition.secret && definition.owner == "core")
        .cloned()
        .collect();
    let mut values = BTreeMap::new();
    for definition in &definitions {
        let setting =
            snapshot
                .get(&definition.key, None)
                .map_err(|source| CoreProjectionError {
                    key: definition.key.clone(),
                    source,
                })?;
        // A default is not a set value: `config_from` applies defaults itself.
        if let (Some(value), Some(level)) = (setting.value, setting.origin.and_then(Origin::level))
        {
            values.insert(definition.key.clone(), (value, level));
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut roles = crate::settings::SettingsRegistry::default();
    for (level, document) in snapshot.documents().levels() {
        for (key, value) in document.clone() {
            if !seen.insert(key.clone()) || store.definition(&key).is_ok() {
                continue;
            }
            let error = |source| CoreProjectionError {
                key: key.clone(),
                source,
            };
            if let Some(role) = role_from_key(&key) {
                if roles.get(&key).is_none() {
                    roles.register(role_definition(&role)).map_err(error)?;
                }
                roles.validate(&key, &value).map_err(error)?;
            } else if let Some((_, field)) = key
                .strip_prefix(keys::AGENTS_PREFIX)
                .and_then(|rest| rest.rsplit_once('.'))
            {
                if field == "enabled" && !value.is_boolean() {
                    return Err(error(SettingsError::InvalidValue));
                }
            } else {
                continue;
            }
            values.insert(key, (value, level));
        }
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestNoSecrets;
    impl crate::settings::SecretStore for TestNoSecrets {
        fn get(&self, _: &str) -> Result<Option<String>, String> {
            Ok(None)
        }
        fn set(&self, _: &str, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn delete(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
    }

    fn values(pairs: &[(&str, Value)]) -> BTreeMap<String, (Value, Level)> {
        pairs
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (value.clone(), Level::Project)))
            .collect()
    }

    #[test]
    fn every_core_setting_is_in_a_category_core_declares() {
        let declared: Vec<_> = categories().into_iter().map(|c| c.id).collect();
        for definition in definitions("demo") {
            assert!(
                declared.contains(&definition.category),
                "{} is in `{}`, which core doesn't declare",
                definition.key,
                definition.category
            );
        }
    }

    #[test]
    fn a_whole_number_written_with_a_zero_fraction_is_that_number() {
        // The schema's `integer` accepts 7.0; so does the typed view.
        let config = config_from(
            "demo",
            &values(&[("oxplow.snapshots.retentionDays", json!(7.0))]),
        )
        .unwrap();
        assert_eq!(config.snapshot_retention_days, 7);
        assert!(check("oxplow.snapshots.retentionDays", &json!(7.5)).is_err());
        assert!(check("oxplow.snapshots.retentionDays", &json!(-1.0)).is_err());
    }

    #[test]
    fn a_file_kind_rule_needs_a_dashed_kind_and_a_glob_that_parses() {
        check(
            keys::FILES_KINDS,
            &json!([{"kind": "lint-config", "glob": "*.toml"}]),
        )
        .unwrap();
        for bad in [
            json!([{"kind": "Lint", "glob": "x"}]),
            json!([{"kind": "a--b", "glob": "x"}]),
            json!([{"kind": "ci", "glob": "a{b"}]),
            json!([{"kind": "ci", "glob": "x", "extra": 1}]),
        ] {
            assert!(check(keys::FILES_KINDS, &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn one_value_is_checked_by_its_own_rule() {
        check(keys::REPLACEMENTS, &json!({"task.board": "oxplow"})).unwrap();
        let refused = check(keys::REPLACEMENTS, &json!({"unknown.target": "x"}))
            .unwrap_err()
            .to_string();
        assert!(refused.contains(keys::REPLACEMENTS), "{refused}");
        check("oxplow.activeProviders.snapshots", &json!("oxplow")).unwrap();
        let refused = check("oxplow.activeProviders.nothing", &json!("oxplow"))
            .unwrap_err()
            .to_string();
        assert!(
            refused.contains("oxplow.activeProviders.nothing"),
            "{refused}"
        );
        // A key core does not own has no core rule.
        check("oxplow.extensions.someone.enabled", &json!(false)).unwrap();
        check("other.limit", &json!("anything")).unwrap();
    }

    #[test]
    fn fixed_fields_are_registered_individually_but_dynamic_maps_are_atomic() {
        let all = definitions("demo");
        let find = |key: &str| all.iter().find(|definition| definition.key == key).unwrap();
        assert_eq!(
            find("oxplow.snapshots.retentionDays").default,
            Some(json!(7))
        );
        assert_eq!(find("oxplow.scripts.memoryMb").default, Some(json!(512)));
        assert_eq!(find("oxplow.generated.exclude").default, Some(json!([])));
        assert_eq!(find("oxplow.ai.roles.default").default, None);
        assert_eq!(find("oxplow.ai.roles.default").category, "ai");
        assert!(all
            .iter()
            .all(|definition| definition.key != "oxplow.scripts"));
        assert!(all.iter().all(|definition| definition.key != "oxplow.ai"));
        assert!(all.iter().all(|definition| !definition.title.is_empty()));
        assert!(all.iter().all(|definition| !definition.category.is_empty()));
        let unique: std::collections::BTreeSet<_> = all.iter().map(|d| &d.key).collect();
        assert_eq!(unique.len(), all.len());
    }

    #[test]
    fn nothing_set_is_every_default() {
        let config = config_from("demo", &BTreeMap::new()).unwrap();
        assert_eq!(config.project_name, "demo");
        assert_eq!(config.snapshot_retention_days, 7);
        assert_eq!(config.snapshot_max_file_bytes, 5 * 1024 * 1024);
        assert_eq!(config.metric_detail_max_per_producer, 100);
        assert_eq!(config.metric_detail_retention_days, 30);
        assert!(config.inject_session_context);
        assert_eq!(config.scripts, crate::ScriptsConfig::default());
        assert_eq!(config.testing, crate::TestingConfig::default());
        assert_eq!(
            config.snapshot_turn_budget_ms,
            crate::DEFAULT_SNAPSHOT_TURN_BUDGET_MS
        );
        assert!(config.agent_enabled.is_empty() && config.active_providers.is_empty());
    }

    #[test]
    fn each_fixed_setting_applies_to_its_own_field() {
        let config = config_from(
            "demo",
            &values(&[
                (keys::PROJECT_NAME, json!("  named ")),
                (keys::ICON_TINT, json!("#abc")),
                (keys::AGENT_PROMPT_APPEND, json!("extra")),
                (keys::INJECT_SESSION_CONTEXT, json!(false)),
                (keys::SNAPSHOT_RETENTION_DAYS, json!(14)),
                (keys::SNAPSHOT_MAX_FILE_BYTES, json!(2048)),
                (keys::METRIC_RETENTION_DAYS, json!(3)),
                (keys::GENERATED_EXCLUDE, json!(["dist/"])),
                (keys::GENERATED_INCLUDE, json!(["vendor"])),
                (keys::TESTING_COMMAND, json!(" make test ")),
                (keys::TESTING_FAST_COMMAND, json!("")),
                (keys::TESTING_RUN_PATTERNS, json!(["pytest"])),
                (keys::TESTING_TEST_PATHS, json!(["spec/**"])),
                (
                    keys::FILES_KINDS,
                    json!([{"kind": "generated", "glob": "gen/**", "label": "Generated"}]),
                ),
                (keys::SCRIPTS_MAX_CONCURRENT, json!(2)),
                (keys::SCRIPTS_MEMORY_MB, json!(256)),
                (keys::METRICS, json!([{"use": "oxplow.loc", "target": 3}])),
                (
                    keys::LSP_SERVERS,
                    json!([{"languageId": "rust", "extensions": [".RS"], "command": "ra"}]),
                ),
                (keys::REPLACEMENTS, json!({"task.board": "oxplow"})),
                (keys::EXTENSION_SOURCES, json!({"oxtask": "../oxtask"})),
                (
                    keys::EXTENSION_INSTANCES,
                    json!({"acme/one": {"enabled": true, "config": {}}}),
                ),
            ]),
        )
        .unwrap();
        assert_eq!(config.project_name, "named");
        assert_eq!(config.icon_tint.as_deref(), Some("#abc"));
        assert_eq!(config.agent_prompt_append, "extra");
        assert!(!config.inject_session_context);
        assert_eq!(config.snapshot_retention_days, 14);
        assert_eq!(config.snapshot_max_file_bytes, 2048);
        assert_eq!(config.metric_retention_days, 3);
        assert_eq!(config.generated.exclude, ["dist"]);
        assert_eq!(config.generated.include, ["vendor"]);
        assert_eq!(config.testing.command.as_deref(), Some("make test"));
        assert_eq!(config.testing.fast_command, None);
        assert_eq!(config.testing.run_patterns, ["pytest"]);
        assert_eq!(config.testing.test_paths, ["spec/**"]);
        assert_eq!(
            config.files.kinds,
            [crate::FileKindRule {
                kind: "generated".into(),
                glob: "gen/**".into(),
                label: Some("Generated".into())
            }]
        );
        assert_eq!(config.scripts.max_concurrent, 2);
        assert_eq!(config.scripts.memory_bytes, 256 * 1024 * 1024);
        assert_eq!(config.metrics[0].use_key.as_deref(), Some("oxplow.loc"));
        assert_eq!(config.lsp_servers[0].extensions, [".rs"]);
        assert_eq!(config.replacements["task.board"], "oxplow");
        assert_eq!(config.extension_sources["oxtask"], "../oxtask");
        assert!(config.extension_instances["acme/one"].enabled);
    }

    #[test]
    fn a_refused_value_names_its_setting_key() {
        for (key, value) in [
            (keys::PROJECT_NAME, json!("   ")),
            (keys::ICON_TINT, json!("blue")),
            (keys::GENERATED_EXCLUDE, json!(["/abs"])),
            (keys::TESTING_TEST_PATHS, json!(["[bad"])),
            (keys::TESTING_RUN_PATTERNS, json!([" "])),
            (keys::REPLACEMENTS, json!({"nope": "oxplow"})),
            (
                keys::LSP_SERVERS,
                json!([{"languageId": "rust", "extensions": ["rs"], "command": "ra"}]),
            ),
            ("oxplow.activeProviders.nope", json!("hashes")),
            (
                "oxplow.ai.roles.fast",
                json!({"provider": "p", "model": "m"}),
            ),
            ("oxplow.agents.Bad.enabled", json!(true)),
        ] {
            let error = config_from("demo", &values(&[(key, value)]))
                .unwrap_err()
                .to_string();
            assert!(error.contains(key), "{key}: {error}");
        }
    }

    #[test]
    fn family_keys_apply_to_their_member() {
        let config = config_from(
            "demo",
            &values(&[
                ("oxplow.activeProviders.snapshots", json!("hashes")),
                (
                    "oxplow.ai.roles.default",
                    json!({"provider": "local", "model": "a"}),
                ),
                ("oxplow.agents.claude.enabled", json!(false)),
                ("oxplow.agents.claude.model", json!("opus")),
                ("oxplow.agents.acme.coder.enabled", json!(false)),
                (
                    &format!(
                        "oxplow.agents.acme.coder.{}",
                        crate::settings::key_segment("api/url")
                    ),
                    json!("https://example.test"),
                ),
                ("oxtask.tasks.workflow", json!("agent_tracks")),
            ]),
        )
        .unwrap();
        assert_eq!(config.active_providers["snapshots"], "hashes");
        assert_eq!(config.active_provider_origins["snapshots"], Level::Project);
        assert!(!config.agent_enabled["claude"]);
        assert!(!config.agent_enabled["acme/coder"]);
        assert_eq!(config.agent_config["claude"]["model"], json!("opus"));
        assert_eq!(
            config.agent_config["acme/coder"]["api/url"],
            json!("https://example.test")
        );
    }

    fn projection_store(
        project: &std::path::Path,
        global: &std::path::Path,
    ) -> crate::settings::SettingsStore {
        let mut registry = crate::settings::SettingsRegistry::default();
        registry.register_many(definitions("demo")).unwrap();
        crate::settings::SettingsStore::new(
            project.to_path_buf(),
            global.to_path_buf(),
            std::sync::Arc::new(std::sync::RwLock::new(registry)),
            std::sync::Arc::new(TestNoSecrets),
        )
    }

    fn write(
        project: &std::path::Path,
        global: &std::path::Path,
        level: Level,
        key: &str,
        value: Option<Value>,
    ) {
        crate::settings::write_layer_key(
            &crate::settings::level_path(project, global, level),
            key,
            value.as_ref(),
        )
        .unwrap();
    }

    #[test]
    fn bootstrap_preserves_unknown_harness_preferences_until_declarations_return() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let store = projection_store(project.path(), global.path());
        let (p, g) = (project.path(), global.path());
        write(
            p,
            g,
            Level::Project,
            "oxplow.agents.acme.coder.enabled",
            Some(json!(false)),
        );
        write(
            p,
            g,
            Level::Project,
            "oxplow.agents.acme.coder.model",
            Some(json!({"name": "custom"})),
        );
        let config = config_from(
            "demo",
            &effective_values(&store.snapshot().unwrap()).unwrap(),
        )
        .unwrap();
        assert!(!config.agent_enabled["acme/coder"]);
        assert_eq!(config.agent_config["acme/coder"]["model"]["name"], "custom");
    }

    #[test]
    fn bootstrap_validation_errors_name_the_setting_key() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let store = projection_store(project.path(), global.path());
        let (p, g) = (project.path(), global.path());
        let key = "oxplow.agents.acme.coder.enabled";
        write(p, g, Level::Project, key, Some(json!("yes")));
        let error = effective_values(&store.snapshot().unwrap())
            .unwrap_err()
            .to_string();
        assert!(error.contains(key), "{error}");
        write(p, g, Level::Project, key, None);
        let role_key = "oxplow.ai.roles.unregistered";
        write(
            p,
            g,
            Level::Project,
            role_key,
            Some(json!({"provider": "local", "model": 42})),
        );
        let error = effective_values(&store.snapshot().unwrap())
            .unwrap_err()
            .to_string();
        assert!(error.contains(role_key), "{error}");
    }

    #[test]
    fn registered_dynamic_value_wins_over_a_lower_level() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let key = "oxplow.agents.acme.coder.enabled";
        let store = projection_store(project.path(), global.path());
        let (p, g) = (project.path(), global.path());
        write(p, g, Level::Project, key, Some(json!(true)));
        write(p, g, Level::Local, key, Some(json!(false)));
        let config = config_from(
            "demo",
            &effective_values(&store.snapshot().unwrap()).unwrap(),
        )
        .unwrap();
        assert!(!config.agent_enabled["acme/coder"]);
    }

    #[test]
    fn capability_preferences_inherit_as_independent_scalar_settings() {
        let project = tempfile::tempdir().unwrap();
        let global = tempfile::tempdir().unwrap();
        let key = "oxplow.activeProviders.snapshots";
        let (p, g) = (project.path(), global.path());
        write(p, g, Level::Global, key, Some(json!("none")));
        write(p, g, Level::Project, key, Some(json!("hashes")));
        write(p, g, Level::Local, key, Some(json!("oxplow")));
        let load = || crate::load_project_config_with_global(p, Some(g)).unwrap();
        let local = load();
        assert_eq!(local.active_providers["snapshots"], "oxplow");
        assert_eq!(local.active_provider_origins["snapshots"], Level::Local);
        write(p, g, Level::Local, key, None);
        let inherited = load();
        assert_eq!(inherited.active_providers["snapshots"], "hashes");
        assert_eq!(
            inherited.active_provider_origins["snapshots"],
            Level::Project
        );
        write(p, g, Level::Project, key, None);
        assert_eq!(load().active_provider_origins["snapshots"], Level::Global);
        assert!(!definitions("demo")
            .iter()
            .any(|definition| definition.key == "oxplow.activeProviders"));
    }

    /// Every core definition's metadata, and which sample values its schema
    /// accepts, as checked in. Rebuilding how core declares its settings must
    /// keep both. `OXPLOW_PIN_UPDATE=1` rewrites the file.
    #[test]
    fn core_definitions_and_accepted_values_are_pinned() {
        let samples = [
            json!(null),
            json!(true),
            json!(false),
            json!(0),
            json!(1),
            json!(-1),
            json!(7),
            json!(64),
            json!(100),
            json!(1024),
            json!(1.5),
            json!(""),
            json!(" "),
            json!("x"),
            json!("hashes"),
            json!("#abc"),
            json!("c2410c"),
            json!([]),
            json!(["a"]),
            json!(["a", "a"]),
            json!(["dist"]),
            json!(["/abs"]),
            json!(["../up"]),
            json!({}),
            json!({"a": "b"}),
            json!({"provider": "p", "model": "m"}),
            json!({"provider": "p", "model": "m", "restrict": ["tools"]}),
            json!([{"use": "oxplow.loc"}]),
            json!([{"use": "oxplow.loc", "target": 3, "enabled": false}]),
            json!([{"languageId": "rust", "extensions": [".rs"], "command": "ra"}]),
            json!([{"label": "core", "match": "src/**"}]),
            json!({"acme/one": {"enabled": true, "config": {}}}),
            json!({"agent": {"payloadDays": 60, "contentDays": 14}}),
            json!({"task.board": "oxplow"}),
            json!({"oxtask": "../oxtask"}),
        ];
        let mut registry = crate::settings::SettingsRegistry::default();
        registry.register_many(definitions("demo")).unwrap();
        let pinned: Vec<Value> = registry
            .definitions()
            .map(|d| {
                let accepts: Vec<usize> = samples
                    .iter()
                    .enumerate()
                    .filter(|(_, sample)| registry.validate(&d.key, sample).is_ok())
                    .map(|(index, _)| index)
                    .collect();
                // The default concurrency depends on the machine's cores.
                let default = if d.key == "oxplow.scripts.maxConcurrent" {
                    json!("<min(4, cores)>")
                } else {
                    json!(d.default)
                };
                json!({
                    "key": d.key, "title": d.title, "description": d.description,
                    "category": d.category, "default": default, "secret": d.secret,
                    "order": d.order, "ui": d.ui,
                    "owner": d.owner, "accepts": accepts,
                })
            })
            .collect();
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/core_settings.json");
        let text = serde_json::to_string_pretty(&pinned).unwrap() + "\n";
        if std::env::var_os("OXPLOW_PIN_UPDATE").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &text).unwrap();
        }
        let expected = std::fs::read_to_string(&path).expect("the pinned core settings file");
        assert_eq!(
            text,
            expected,
            "core settings changed; see {}",
            path.display()
        );
    }

    #[test]
    fn every_core_default_validates_and_applies() {
        let mut registry = crate::settings::SettingsRegistry::default();
        registry.register_many(definitions("demo")).unwrap();
        config_from("demo", &BTreeMap::new()).unwrap();
    }
}
