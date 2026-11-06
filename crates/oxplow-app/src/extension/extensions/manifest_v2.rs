//! `extension.yaml`, manifest version 2 (`.context/extensions-manifest.md`).
//!
//! A v2 manifest declares every contribution as data, carries the
//! extension's **intent**, says whether it is **private** or **shared**, and
//! names the kinds it uses; unknown keys are errors. A manifest without
//! `manifest: 2` doesn't load.
//!
//! This module parses and checks the manifest's own shape and
//! lifecycle; the loader (`extensions.rs`) resolves cross-references
//! (a slot mount to its page, a metric to its measure) with what it
//! loads.

use oxplow_domain::diagnostic::Diagnostic;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;

/// The manifest version this loader writes and prefers.
pub const CURRENT: u32 = 2;

/// One acceptance example: an input and what the extension should make
/// of it. Fixtures for `oxplow extension test`; data here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntentExample {
    pub name: String,
    #[serde(default)]
    #[specta(type = specta_typescript::Unknown)]
    pub input: serde_json::Value,
    #[serde(default)]
    #[specta(type = specta_typescript::Unknown)]
    pub expect: serde_json::Value,
}

/// Why the extension exists — what makes it regenerable, repairable and
/// reviewable against what it was for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    /// The question it answers, or the job it does.
    pub purpose: String,
    /// The thread or effort ref that created it (`oxplow:oxplow/effort?id=eff42`), when known.
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub examples: Vec<IntentExample>,
}

/// `intent:` as written: its examples are typed one by one
/// ([`intent_of`]), so a broken one is that entry's error and the rest
/// of the extension still loads. Its schema is [`Intent`]'s.
#[derive(Debug, Clone, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(with = "Intent")]
pub struct IntentFile {
    pub purpose: String,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub examples: Vec<Value>,
}

/// A mount: a page this extension places — at a page's slot
/// (`at: <extension>/<page>.<slot>`) or in place of a core component
/// (`at: <replaceable target>`, `v_replaceable`). One list, `ui.mounts`.
#[derive(Debug, Clone, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Mount {
    /// The page it places: a slug of this extension.
    pub page: String,
    /// Where: a page's slot (`oxplow-foundation/commit.sections`) or a
    /// replaceable core component (`work_item.board`).
    pub at: String,
    /// At a parts slot: where among the page's parts — `start`, `end`,
    /// `{ after: <id> }`, `{ before: <id> }`. Default `end`.
    #[serde(default)]
    pub position: Option<super::SlotAt>,
    /// At a slot: its heading in the page; else the page's title.
    #[serde(default)]
    pub title: Option<String>,
    /// At a `.tiles` slot: the tile's size.
    #[serde(default)]
    pub size: Option<super::TileSize>,
    /// In place of a core component: the props contract's version it was
    /// written against.
    #[serde(default)]
    pub version: Option<u32>,
}

/// A mount at a page's slot, sorted out of `ui.mounts`.
#[derive(Debug, Clone, PartialEq)]
pub struct SlotMount {
    /// A page's slot (`<extension>/<page>.<slot>`).
    pub slot: String,
    /// The page it mounts: a slug of this extension.
    pub page: String,
    /// Where among the page's parts (a parts slot only): `start`, `end`,
    /// `{ after: <id> }`, `{ before: <id> }`. Default `end`.
    pub at: Option<super::SlotAt>,
    /// Its heading in the page; else the mounted page's title.
    pub title: Option<String>,
    /// A tile's size (a `.tiles` slot only).
    pub size: Option<super::TileSize>,
}

/// `ui:` — everything an extension adds to the core UI: its pages placed
/// at other pages' slots or in place of core components (`mounts`) and
/// decorations on core refs (`decorators`). Its commands meet a person
/// through their own `ui` (`commands:`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiBlock {
    /// Its pages placed elsewhere: `{ page, at, … }`, typed one by one
    /// ([`mounts_of`]).
    #[serde(default)]
    #[schemars(with = "Vec<Mount>")]
    pub mounts: Vec<Value>,
    /// Labels from its datasets on core refs.
    #[serde(default)]
    #[schemars(with = "Option<Vec<super::decorators::DecoratorFile>>")]
    pub decorators: Option<Value>,
}

/// `extension.yaml` at `manifest: 2`, as written on disk.
#[derive(Debug, Clone, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ManifestV2 {
    /// The manifest version: `2`.
    pub manifest: u32,
    /// The extension's name; must equal its folder's.
    pub name: String,
    /// The namespace its commands' ids are under
    /// (`<namespace>.<area>.<verb>`); defaults to the name with `-` → `_`.
    /// `oxplow` is reserved for the extensions that ship with oxplow.
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub description: String,
    /// The oxplow version range it targets, `>=MAJOR.MINOR[.PATCH]`.
    #[serde(default)]
    pub engine: Option<String>,
    /// Its own version, `MAJOR.MINOR.PATCH` (semver; a `-pre` suffix is
    /// fine). Recorded on every contribution it makes.
    #[serde(default)]
    pub version: Option<String>,
    /// Why it exists: its purpose, the ref that created it, and examples.
    /// Required.
    #[schemars(required)]
    pub intent: Option<IntentFile>,

    /// Explicit versioned contracts offered to other extensions.
    #[serde(default)]
    pub exports: Vec<super::public_contracts::ExportFile>,
    /// Consumer-local bindings to canonical public contributions.
    #[serde(default)]
    pub imports: Vec<super::public_contracts::ImportFile>,
    /// Contributions to withhold when one of these bindings is unavailable.
    #[serde(default)]
    pub import_guards: Vec<super::public_contracts::ImportGuardFile>,

    /// Versioned non-secret settings schema and partial defaults. Read raw so
    /// declaration errors never format configuration values into diagnostics.
    #[serde(default, deserialize_with = "present_configuration")]
    #[schemars(
        with = "super::config_schema::ConfigurationFile",
        skip_serializing_if = "Option::is_none"
    )]
    pub configuration: Option<Value>,
    /// The other extensions it needs on: deploying or turning it on refuses while one is off,
    /// and turning one off refuses while this is on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    /// A command a deploy runs in the source folder before it copies.
    /// Read raw so a broken block is
    /// its own error at its line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<Build>")]
    pub build: Option<Value>,
    /// What it may reach beyond the baseline:
    /// checked against what it contains when it loads,
    /// approved when it is deployed.
    #[serde(
        default,
        skip_serializing_if = "oxplow_domain::permissions::Permissions::is_empty"
    )]
    pub permissions: oxplow_domain::permissions::Permissions,

    // ---- kinds ----
    /// Datasets: each with its SQL in `datasets/<name>.sql`, published as
    /// `v_<extension>_<name>`.
    #[serde(default)]
    #[schemars(with = "Option<Vec<oxplow_db::datasets::DatasetDecl>>")]
    pub datasets: Option<Value>,
    /// Measures, in `.oxplow/project.yaml`'s vocabulary.
    #[serde(default)]
    #[schemars(with = "Option<Vec<oxplow_config::MeasureEntry>>")]
    pub measures: Option<Value>,
    /// Metric definitions (`key:`), in `.oxplow/project.yaml`'s vocabulary.
    #[serde(default)]
    #[schemars(with = "Option<Vec<oxplow_config::MetricEntry>>")]
    pub metrics: Option<Value>,
    /// Dimensions, in `.oxplow/project.yaml`'s vocabulary.
    #[serde(default)]
    #[schemars(with = "Option<Vec<oxplow_config::DimensionEntry>>")]
    pub dimensions: Option<Value>,
    /// Collectors (exec / starlark / jaq / read), writing entities or
    /// recording facts.
    #[serde(default)]
    #[schemars(with = "Option<Vec<oxplow_config::collectors::RawCollector>>")]
    pub collectors: Option<Value>,
    /// Commands: a Starlark script composing core commands, a scope's
    /// operation, or one of its provider's.
    #[serde(default)]
    #[schemars(with = "Option<Vec<crate::extension::extension_commands::CommandFile>>")]
    pub commands: Option<Value>,
    /// Durable host operations that an extension may schedule.
    #[serde(default)]
    #[schemars(with = "Option<Vec<super::jobs::JobFile>>")]
    pub jobs: Option<Value>,
    /// Named dataset needs: the person binds `<extension>/<id>` to a dataset,
    /// else the fallback role's dataset serves it.
    #[serde(default)]
    #[schemars(with = "Option<Vec<super::ai_profiles::AiProfileFile>>")]
    pub ai_profiles: Option<Value>,
    /// Left-nav panels, each showing its pages compact.
    #[serde(default)]
    #[schemars(with = "Option<Vec<super::PanelFile>>")]
    pub panels: Option<Value>,
    /// What it adds to the core UI: slots, decorators, replacements.
    #[serde(default)]
    pub ui: UiBlock,
    /// Guidance queries run at a moment (`on`), whose rows reach the
    /// agent or a person.
    #[serde(default)]
    #[schemars(with = "Vec<super::AdvisoryFile>")]
    pub advisories: Vec<Value>,
    /// The event types it may log, under its own namespace.
    #[serde(default)]
    #[schemars(with = "Option<crate::extension::extension_event_types::EventTypesFile>")]
    pub event_types: Option<Value>,
    /// What its URIs may name (`oxplow:<extension>/<resource>`).
    #[serde(default)]
    #[schemars(with = "Option<Vec<crate::extension::extension_resources::ResourceFile>>")]
    pub resources: Option<Value>,
    /// Capability implementations: built-ins of core's it declares.
    #[serde(default)]
    #[schemars(with = "Option<Vec<super::implementations::ImplementationFile>>")]
    pub implementations: Option<Value>,
    /// Skills and slash commands for the coding agent.
    #[serde(default)]
    #[schemars(with = "Option<Vec<super::skills::SkillFile>>")]
    pub skills: Option<Value>,
    /// External providers: programs implementing a capability, whose
    /// operations its `commands:` declare.
    #[serde(default)]
    #[schemars(with = "Option<Vec<crate::providers::spec::ProviderSpec>>")]
    pub providers: Option<Value>,
    /// Scripts reacting to logged events by composing commands.
    #[serde(default)]
    #[schemars(with = "Option<Vec<crate::effects::EffectFile>>")]
    pub effects: Option<Value>,
    /// Sandboxed web components for `viz: custom` viz nodes.
    #[serde(default)]
    #[schemars(with = "Option<Vec<super::custom_components::ComponentFile>>")]
    pub custom_components: Option<Value>,
}

/// The 1-based line of a top-level `key:` in a YAML document, for
/// `file:line` messages. `None` when the key isn't written (a default).
pub fn key_line(text: &str, key: &str) -> Option<usize> {
    text.lines().enumerate().find_map(|(i, line)| {
        let rest = line.strip_prefix(key)?;
        rest.starts_with(':').then_some(i + 1)
    })
}

/// The 1-based line of the first line containing `needle` after the
/// top-level `key:` — a mount's page name inside `slot_mounts`, say.
pub fn line_under(text: &str, key: &str, needle: &str) -> Option<usize> {
    let start = key_line(text, key)?;
    text.lines()
        .enumerate()
        .skip(start)
        .take_while(|(_, l)| l.starts_with(' ') || l.starts_with('-') || l.trim().is_empty())
        .find(|(_, l)| l.contains(needle))
        .map(|(i, _)| i + 1)
}

/// The 1-based line, inside top-level `block`, of the entry whose `key`
/// is exactly `value` (`key: value`, `key: "value"`, `{ key: value, … }`)
/// — not one whose value merely starts with it.
pub fn entry_line(text: &str, block: &str, key: &str, value: &str) -> Option<usize> {
    let start = key_line(text, block)?;
    let pattern = format!("{key}:");
    let names_it = |line: &str| {
        line.match_indices(&pattern).any(|(at, _)| {
            let before = line[..at].chars().next_back();
            if !before.is_none_or(|c| matches!(c, ' ' | '{' | ',' | '-')) {
                return false;
            }
            let rest = line[at + pattern.len()..].trim_start();
            let (quote, rest) = match rest.chars().next() {
                Some(q @ ('"' | '\'')) => (Some(q), &rest[1..]),
                _ => (None, rest),
            };
            let Some(after) = rest.strip_prefix(value) else {
                return false;
            };
            match quote {
                Some(q) => after.starts_with(q),
                None => after.is_empty() || after.starts_with([' ', ',', '}', '#']),
            }
        })
    };
    text.lines()
        .enumerate()
        .skip(start)
        .take_while(|(_, l)| l.starts_with(' ') || l.starts_with('-') || l.trim().is_empty())
        .find(|(_, l)| names_it(l))
        .map(|(i, _)| i + 1)
}

/// The line of a kind's key: a top-level key, or `<key>.<sub>` — a key
/// under a top-level one (`ui.decorators`, `event_types.types`).
pub fn kind_line(text: &str, kind: &str) -> Option<usize> {
    match kind.split_once('.') {
        Some((top, sub)) => line_under(text, top, &format!("{sub}:")).or(key_line(text, top)),
        None => key_line(text, kind),
    }
}

/// The 1-based line of each item of the block list `kind` (a top-level
/// key, or `<key>.<sub>`): where an entry's own error goes — a shape error
/// included, which has no name to look the entry up by. Empty for a flow
/// list (`[a, b]`) or a block that isn't a list; the caller falls back
/// to the block's line.
pub fn item_lines(text: &str, kind: &str) -> Vec<usize> {
    let indent = |l: &str| l.len() - l.trim_start().len();
    let lines: Vec<&str> = text.lines().collect();
    let Some(key) = kind_line(text, kind) else {
        return Vec::new();
    };
    let key_indent = indent(lines[key - 1]);
    let mut out = Vec::new();
    let mut item_indent = None;
    for (i, line) in lines.iter().enumerate().skip(key) {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let at = indent(line);
        // A sibling key at the block's own indent ends it; a list under a
        // top-level key may sit at that indent (`key:\n- a`).
        if at < key_indent || (at == key_indent && !trimmed.starts_with('-')) {
            break;
        }
        if trimmed.starts_with("- ") || trimmed == "-" {
            match item_indent {
                None => {
                    item_indent = Some(at);
                    out.push(i + 1);
                }
                Some(n) if at == n => out.push(i + 1),
                _ => {}
            }
        }
    }
    out
}

/// Each entry of list `kind` (`intent.examples`, `ui.slots`, …) typed as
/// `T`: the good ones, and an error at its own line for each broken one.
fn entries<T: serde::de::DeserializeOwned>(
    items: &[Value],
    kind: &str,
    file: &str,
    text: &str,
) -> (Vec<T>, Vec<Diagnostic>) {
    let lines = item_lines(text, kind);
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for (i, item) in items.iter().enumerate() {
        match serde_yaml::from_value(item.clone()) {
            Ok(t) => out.push(t),
            Err(e) => {
                let line = lines.get(i).copied().or(kind_line(text, kind));
                errors.push(at(
                    "manifest.invalid_entry",
                    file,
                    line,
                    format!("`{kind}`: {e}"),
                ));
            }
        }
    }
    (out, errors)
}

/// The manifest's intent, its examples typed one by one: a
/// broken one is left out and is an error at its line.
pub fn intent_of(m: &ManifestV2, file: &str, text: &str) -> (Option<Intent>, Vec<Diagnostic>) {
    let Some(raw) = &m.intent else {
        return (None, Vec::new());
    };
    let (examples, errors) = entries(&raw.examples, "intent.examples", file, text);
    let intent = Intent {
        purpose: raw.purpose.clone(),
        origin: raw.origin.clone(),
        examples,
    };
    (Some(intent), errors)
}

/// The manifest's `ui.mounts`, typed one by one and sorted by what `at`
/// names: a page's slot (it has a `/`) or a core component. Each comes
/// with its own line; what's wrong with one is an error at it.
pub fn mounts_of(m: &ManifestV2, file: &str, text: &str) -> Mounts {
    let lines = item_lines(text, "ui.mounts");
    let block = line_under(text, "ui", "mounts:").or(key_line(text, "ui"));
    let mut out = Mounts::default();
    for (i, item) in m.ui.mounts.iter().enumerate() {
        let line = lines.get(i).copied().or(block);
        let mount: Mount = match serde_yaml::from_value(item.clone()) {
            Ok(mount) => mount,
            Err(e) => {
                out.errors
                    .push(at("mount.invalid", file, line, format!("`ui.mounts`: {e}")));
                continue;
            }
        };
        if mount.at.contains('/') {
            if mount.version.is_some() {
                out.errors.push(at(
                    "mount.version_on_slot",
                    file,
                    line,
                    format!(
                        "`ui.mounts`: `version` goes with a core component; `{}` is a page's slot",
                        mount.at
                    ),
                ));
                continue;
            }
            out.slots.push((
                line,
                SlotMount {
                    slot: mount.at,
                    page: mount.page,
                    at: mount.position,
                    title: mount.title,
                    size: mount.size,
                },
            ));
        } else {
            if mount.position.is_some() || mount.title.is_some() || mount.size.is_some() {
                out.errors.push(at(
                    "mount.slot_key_on_component",
                    file,
                    line,
                    format!(
                        "`ui.mounts`: `position`, `title` and `size` go with a page's slot; `{}` is a core component",
                        mount.at
                    ),
                ));
                continue;
            }
            out.replacements.push((
                line,
                super::replacements::ReplacementFile::new(mount.at, mount.page, mount.version),
            ));
        }
    }
    out
}

/// `ui.mounts`, sorted: slot mounts and replacements, each with its line,
/// in the order written; and what was wrong with the rest.
#[derive(Default)]
pub struct Mounts {
    pub slots: Vec<(Option<usize>, SlotMount)>,
    pub replacements: Vec<(Option<usize>, super::replacements::ReplacementFile)>,
    pub errors: Vec<Diagnostic>,
}

/// An error `code` in `file` (relative to the extension's folder), at
/// `line` when known.
pub fn at(
    code: &str,
    file: &str,
    line: Option<usize>,
    message: impl std::fmt::Display,
) -> Diagnostic {
    Diagnostic::error(code, message.to_string()).at(file, line)
}

/// Does the running oxplow satisfy `engine`? Only `>=MAJOR.MINOR[.PATCH]`
/// is accepted, so the rule stays readable to an agent and a person.
pub fn engine_check(engine: &str, current: &str) -> Result<bool, String> {
    let req = engine
        .trim()
        .strip_prefix(">=")
        .ok_or_else(|| format!("engine `{engine}` must be `>=MAJOR.MINOR[.PATCH]`"))?
        .trim();
    let parse = |s: &str| -> Result<(u64, u64, u64), String> {
        let mut parts = s.split('.').map(|p| {
            p.parse::<u64>()
                .map_err(|_| format!("engine `{engine}` must be `>=MAJOR.MINOR[.PATCH]`"))
        });
        let major = parts
            .next()
            .ok_or_else(|| format!("engine `{engine}` is empty"))??;
        let minor = parts.next().transpose()?.unwrap_or(0);
        let patch = parts.next().transpose()?.unwrap_or(0);
        if parts.next().is_some() {
            return Err(format!("engine `{engine}` has too many components"));
        }
        Ok((major, minor, patch))
    };
    let required = parse(req)?;
    let have = parse(current).map_err(|_| format!("oxplow version `{current}` unreadable"))?;
    Ok(have >= required)
}

/// The oxplow version the loader runs in.
pub fn current_engine() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// An absent declaration is optional; an explicitly present null is invalid.
fn present_configuration<'de, D>(deserializer: D) -> Result<Option<Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Value::deserialize(deserializer).map(Some)
}

/// What a deploy builds before it copies: `run` in the source folder, as
/// the person, then `output` is copied into the deployment. The hash is of
/// the inputs — every file but `output` and the `exclude` folders — and
/// `run` itself, so a build's output never changes what is deployed.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Build {
    /// The command, run with `/bin/sh -c` in the source folder with the
    /// person's environment. A new or changed one is approved at deploy.
    pub run: String,
    /// The folder it writes (`dist`), relative to the extension: copied
    /// into the deployment, never hashed.
    pub output: String,
    /// Folders neither hashed nor deployed (`node_modules`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
}

/// The manifest's `build:`, checked: errors at its line.
pub fn build_of(m: &ManifestV2, file: &str, text: &str) -> (Option<Build>, Vec<Diagnostic>) {
    let Some(raw) = &m.build else {
        return (None, Vec::new());
    };
    let line = key_line(text, "build");
    let build = match serde_yaml::from_value::<Build>(raw.clone()) {
        Ok(build) => build,
        Err(e) => {
            return (
                None,
                vec![at(
                    "manifest.invalid_build",
                    file,
                    line,
                    format!("build: {e}"),
                )],
            )
        }
    };
    let mut errors = Vec::new();
    if build.run.trim().is_empty() {
        errors.push(at(
            "manifest.build_run_empty",
            file,
            line,
            "build: `run` is empty",
        ));
    }
    for dir in std::iter::once(&build.output).chain(&build.exclude) {
        let inside = oxplow_domain::service::valid_path(dir)
            && !dir.split('/').any(|part| part == "extension.yaml");
        if !inside {
            errors.push(at(
                "manifest.build_dir_outside",
                file,
                line,
                format!("build: `{dir}` isn't a folder inside the extension"),
            ));
        }
    }
    let trim = |d: &str| d.trim_end_matches('/').to_string();
    if build.exclude.iter().any(|d| trim(d) == trim(&build.output)) {
        errors.push(at(
            "manifest.build_output_excluded",
            file,
            line,
            format!(
                "build: `{}` is the output; it can't be excluded too",
                build.output
            ),
        ));
    }
    if errors.is_empty() {
        (Some(build), errors)
    } else {
        (None, errors)
    }
}

/// Parse and validate settings without echoing raw YAML/JSON in errors.
/// Required fields deferred in partial defaults are checked on final resolution.
pub fn configuration_of(
    m: &ManifestV2,
    file: &str,
    text: &str,
) -> (
    Option<super::config_schema::ConfigurationFile>,
    Vec<Diagnostic>,
) {
    let Some(raw) = &m.configuration else {
        return (None, Vec::new());
    };
    let line = key_line(text, "configuration");
    let parsed = serde_yaml::from_value::<super::config_schema::ConfigurationFile>(raw.clone());
    let declaration = match parsed {
        Ok(declaration) => declaration,
        Err(_) => {
            return (
                None,
                vec![at(
                    "config.invalid_declaration",
                    file,
                    line,
                    "configuration: invalid_declaration",
                )],
            )
        }
    };
    match declaration.prepare() {
        Ok(_) => (Some(declaration), Vec::new()),
        Err(issues) => (
            None,
            issues
                .into_iter()
                .map(|issue| {
                    at(
                        "config.invalid_declaration",
                        file,
                        line,
                        format!("configuration: {} at {}", issue.code, issue.pointer),
                    )
                })
                .collect(),
        ),
    }
}

/// The errors in a manifest's own shape and lifecycle (not its
/// cross-references, which need the loaded pages). `file` is how the
/// manifest is named in messages; `text` its source, for lines.
pub fn check(m: &ManifestV2, file: &str, text: &str) -> Vec<Diagnostic> {
    let mut errors = Vec::new();
    errors.extend(configuration_of(m, file, text).1);
    if m.manifest != CURRENT {
        errors.push(at(
            "manifest.unsupported_version",
            file,
            key_line(text, "manifest"),
            format!(
                "manifest version {} is not supported; this oxplow reads {CURRENT}",
                m.manifest
            ),
        ));
    }
    let (intent, intent_errors) = intent_of(m, file, text);
    errors.extend(intent_errors);
    match &intent {
        None => errors.push(
            at(
                "intent.required",
                file,
                Some(1),
                "`intent` is required",
            )
            .fix("add `intent: { purpose: <what it is for>, origin: <thread or effort ref>, examples: [...] }`"),
        ),
        Some(intent) => {
            if intent.purpose.trim().is_empty() {
                errors.push(at("intent.purpose_empty", file, key_line(text, "intent"), "`intent.purpose` is empty"));
            }
            if let Some(origin) = &intent.origin {
                if oxplow_domain::uri::OxplowUri::parse(origin).is_err() {
                    errors.push(
                        at(
                            "intent.origin_not_uri",
                            file,
                            key_line(text, "intent"),
                            format!("`intent.origin` `{origin}` is not an oxplow URI"),
                        )
                        .fix("use `oxplow:oxplow/effort?id=eff42` or `oxplow:oxplow/thread?id=thr3`"),
                    );
                }
            }
        }
    }
    // An `engine` it names must be met by this oxplow.
    if let Some(engine) = &m.engine {
        match engine_check(engine, current_engine()) {
            Ok(true) => {}
            Ok(false) => errors.push(at(
                "manifest.engine_unmet",
                file,
                key_line(text, "engine"),
                format!("needs oxplow {engine}; this is {}", current_engine()),
            )),
            Err(e) => errors.push(at(
                "manifest.invalid_engine",
                file,
                key_line(text, "engine"),
                e,
            )),
        }
    }
    match &m.version {
        Some(v) if !valid_version(v) => errors.push(at(
            "manifest.invalid_version",
            file,
            key_line(text, "version"),
            format!("`version: {v}` must be MAJOR.MINOR.PATCH (a `-pre` suffix is fine)"),
        )),
        _ => {}
    }
    errors
}

/// `MAJOR.MINOR.PATCH`, numbers each, with an optional `-pre` suffix.
pub fn valid_version(v: &str) -> bool {
    let core = v
        .split_once('-')
        .map_or(v, |(core, pre)| if pre.is_empty() { "" } else { core });
    let parts: Vec<&str> = core.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> ManifestV2 {
        serde_yaml::from_str(text).unwrap()
    }

    const OK: &str = "manifest: 2\nname: acme\nintent:\n  purpose: Count things\n  examples:\n    - { name: one, input: {}, expect: {} }\n";

    #[test]
    fn a_minimal_manifest_is_clean() {
        let m = parse(OK);
        let errors = check(&m, "ext/extension.yaml", OK);
        assert!(errors.is_empty(), "{errors:?}");
    }

    /// `intent` is required. Its examples may be empty here: fixture files
    /// are examples too, so whether it has none is the load's to say
    /// (`fixtures_are_examples`), where the folder is.
    #[test]
    fn intent_is_required() {
        let text = "manifest: 2\nname: acme\n";
        let errors = check(&parse(text), "e/extension.yaml", text);
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("`intent` is required")),
            "{errors:?}"
        );
        let text = "manifest: 2\nname: acme\nintent:\n  purpose: x\n";
        let errors = check(&parse(text), "e/extension.yaml", text);
        assert!(errors.is_empty(), "{errors:?}");
    }

    /// `permissions:` is checked as it parses: a misspelled or unknown
    /// grant is an error, never silently nothing.
    #[test]
    fn permissions_parse_and_an_unknown_grant_is_an_error() {
        let text = "manifest: 2\nname: acme\npermissions: [process.run, network:api.github.com]\nintent:\n  purpose: x\n";
        let m = parse(text);
        assert!(m.permissions.process);
        assert_eq!(m.permissions.network, vec!["api.github.com".to_string()]);
        assert!(parse(OK).permissions.is_empty());
        let text = "manifest: 2\nname: acme\npermissions: [proces.run]\nintent:\n  purpose: x\n";
        assert!(serde_yaml::from_str::<ManifestV2>(text).is_err());
        // The map it used to be is refused too.
        let text =
            "manifest: 2\nname: acme\npermissions:\n  process: true\nintent:\n  purpose: x\n";
        assert!(serde_yaml::from_str::<ManifestV2>(text).is_err());
    }

    #[test]
    fn an_engine_named_must_be_met() {
        let text = "manifest: 2\nname: acme\nengine: \">=0.1\"\nversion: 0.1.0\nintent:\n  purpose: x\n  examples: [{ name: a }]\n";
        let errors = check(&parse(text), "e/extension.yaml", text);
        assert!(errors.is_empty(), "{errors:?}");
        let text = "manifest: 2\nname: acme\nengine: \">=99.0\"\nintent:\n  purpose: x\n  examples: [{ name: a }]\n";
        let errors = check(&parse(text), "e/extension.yaml", text);
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("needs oxplow >=99.0")),
            "{errors:?}"
        );
    }

    /// Each list item's own line, under a top-level key or under `ui:`,
    /// nested lists and other keys aside.
    #[test]
    fn each_items_line_is_found() {
        let text = "manifest: 2\ncommands:\n  - name: a\n    examples:\n      - { name: x }\n  # gone\n  - name: b\nui:\n  slots: []\n  decorators:\n    - { dataset: m }\n    - { dataset: n }\neffects:\n- id: e\n";
        assert_eq!(item_lines(text, "commands"), vec![3, 7]);
        assert_eq!(item_lines(text, "ui.decorators"), vec![11, 12]);
        assert_eq!(item_lines(text, "effects"), vec![14]);
        assert!(item_lines(text, "ui.slots").is_empty());
        assert!(item_lines(text, "skills").is_empty());
    }

    #[test]
    fn unknown_keys_and_bad_versions_are_errors() {
        assert!(serde_yaml::from_str::<ManifestV2>("manifest: 2\nname: a\nslots: []\n").is_err());
        // Nothing reads an extension-wide `config:`: an instance's config
        // schema is its provider's declarations'.
        assert!(serde_yaml::from_str::<ManifestV2>("manifest: 2\nname: a\nconfig: {}\n").is_err());
        let text = "manifest: 3\nname: acme\nintent: { purpose: x, examples: [{ name: a }] }\n";
        let errors = check(&parse(text), "e/extension.yaml", text);
        assert!(
            errors[0]
                .message
                .contains("manifest version 3 is not supported"),
            "{errors:?}"
        );
        let text = "manifest: 2\nname: acme\nintent: { purpose: x, origin: nope, examples: [{ name: a }] }\n";
        let errors = check(&parse(text), "e/extension.yaml", text);
        assert!(
            errors[0].message.contains("is not an oxplow URI"),
            "{errors:?}"
        );
    }

    #[test]
    fn engine_ranges() {
        assert_eq!(engine_check(">=0.7", "0.7.0"), Ok(true));
        assert_eq!(engine_check(">=0.7.1", "0.7.0"), Ok(false));
        assert_eq!(engine_check(">= 0.6", "0.7.0"), Ok(true));
        assert!(engine_check("^0.7", "0.7.0").is_err());
        assert!(engine_check(">=a.b", "0.7.0").is_err());
    }

    #[test]
    fn key_lines_are_one_based_and_nested_lookups_stay_in_their_block() {
        let text = "manifest: 2\nname: a\nslot_mounts:\n  - { page: x, at: rail }\n  - { page: y, at: commit }\nadvisories: []\n";
        assert_eq!(key_line(text, "slot_mounts"), Some(3));
        assert_eq!(line_under(text, "slot_mounts", "page: y"), Some(5));
        assert_eq!(line_under(text, "slot_mounts", "advisories"), None);
        assert_eq!(key_line(text, "nope"), None);
    }

    /// An entry's line is the one whose `key:` is exactly that value, in
    /// flow or block style, bare or quoted — `name: a` isn't `name: abc`.
    #[test]
    fn an_entry_line_matches_its_value_exactly() {
        let text = "manifest: 2\ncommands:\n  - name: abc\n    summary: s\n  - name: a\n  - { name: \"b\", x: 1 }\nother: 1\n";
        assert_eq!(entry_line(text, "commands", "name", "a"), Some(5));
        assert_eq!(entry_line(text, "commands", "name", "abc"), Some(3));
        assert_eq!(entry_line(text, "commands", "name", "b"), Some(6));
        assert_eq!(entry_line(text, "commands", "name", "ab"), None);
        assert_eq!(entry_line(text, "commands", "summary", "s"), Some(4));
    }
}
