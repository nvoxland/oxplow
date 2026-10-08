//! `extension.yaml`, manifest version 2 (`.context/extensions.md`).
//!
//! A v2 manifest declares every contribution as data, carries the
//! extension's **intent**, says whether it is **private** or **shared**, and
//! names the kinds it uses; unknown keys are errors. Kinds have a
//! lifecycle: the stable ones are permanent API, the experimental ones
//! may appear only in a private extension. A manifest without
//! `manifest: 2` doesn't load.
//!
//! This module parses and checks the manifest's own shape and
//! lifecycle; the loader (`extensions.rs`) resolves cross-references
//! (a slot mount to its lens, a metric to its measure) with what it
//! loads.

use serde::{Deserialize, Serialize};
use serde_yaml::Value;

/// The manifest version this loader writes and prefers.
pub const CURRENT: u32 = 2;

/// Who the extension is for. Explicit and checked: a shared extension
/// (committed for a team, installed from git, bundled) may use stable
/// kinds only and must name the engine it targets.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    specta::Type,
    Default,
    schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Sharing {
    #[default]
    Private,
    Shared,
}

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
    /// The thread or effort ref that created it (`effort:eff42`), when known.
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub examples: Vec<IntentExample>,
    /// Questions it helps answer, offered to the person with an Ask
    /// button (the catalog; pages for a ref of `about`'s kind).
    #[serde(default)]
    pub prompts: Vec<IntentPrompt>,
}

/// `intent:` as written: its examples and prompts are typed one by one
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
    #[serde(default)]
    pub prompts: Vec<Value>,
}

/// A question an extension helps answer. `about` is a ref kind
/// (`commit`, `file`, `effort`): a page for a ref of that kind suggests it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntentPrompt {
    pub prompt: String,
    #[serde(default)]
    pub about: Option<String>,
}

/// A slot mount: a lens into a core page.
#[derive(Debug, Clone, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SlotMount {
    pub slot: String,
    pub lens: String,
}

/// `ui:` — everything an extension adds to the core UI: lenses mounted
/// into core pages (`slots`), decorations on core refs (`decorators`) and
/// replaced sub-components (`replacements`, experimental). Its commands
/// meet a person through their own `ui` (`commands:`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiBlock {
    /// Lenses mounted into core pages: `{ slot, lens }`, typed one by one
    /// ([`slots_of`]).
    #[serde(default)]
    #[schemars(with = "Vec<SlotMount>")]
    pub slots: Vec<Value>,
    /// Labels from its models on core refs.
    #[serde(default)]
    #[schemars(with = "Option<Vec<super::decorators::DecoratorFile>>")]
    pub decorators: Option<Value>,
    /// Experimental (a private extension only): a lens in place of a
    /// named core component.
    #[serde(default)]
    #[schemars(with = "Option<Vec<super::replacements::ReplacementFile>>")]
    pub replacements: Option<Value>,
}

/// A prompt an extension offers is inserted into the agent's input, never
/// sent — and the terminal treats a pasted line break as Enter, so a
/// multi-line prompt would send itself. One line, or this says why not.
fn prompt_line_problem(prompt: &str) -> Option<String> {
    prompt.contains(['\n', '\r']).then(|| {
        "a prompt is one line (a line break pasted into the agent's input would send it)"
            .to_string()
    })
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
    /// `private` (the default: this project, experimental kinds allowed)
    /// or `shared` (stable kinds only, `engine` required).
    #[serde(default)]
    pub sharing: Sharing,
    /// The oxplow version range it targets, `>=MAJOR.MINOR[.PATCH]`.
    /// Required when shared.
    #[serde(default)]
    pub engine: Option<String>,
    /// Why it exists: its purpose, the ref that created it, and examples.
    /// Required.
    #[schemars(required)]
    pub intent: Option<IntentFile>,

    // ---- stable kinds ----
    /// SQL models: each with `models/<name>.sql`, published as
    /// `v_<extension>_<name>`.
    #[serde(default)]
    #[schemars(with = "Option<Vec<oxplow_db::models::ModelDecl>>")]
    pub models: Option<Value>,
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
    #[schemars(with = "Option<Vec<crate::extension_commands::CommandFile>>")]
    pub commands: Option<Value>,
    /// Full pages, each showing one of its lenses.
    #[serde(default)]
    #[schemars(with = "Option<Vec<super::PageFile>>")]
    pub pages: Option<Value>,
    /// Left-nav panels, each showing its lenses compact.
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
    #[schemars(with = "Option<crate::extension_event_types::EventTypesFile>")]
    pub event_types: Option<Value>,
    /// Kinds of thing a ref can name.
    #[serde(default)]
    #[schemars(with = "Option<Vec<crate::extension_ref_kinds::RefKindFile>>")]
    pub ref_kinds: Option<Value>,
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
    /// Sandboxed web components for `viz: custom` lenses.
    #[serde(default)]
    #[schemars(with = "Option<Vec<super::custom_components::ComponentFile>>")]
    pub custom_components: Option<Value>,
}

/// The kinds a shared extension may not use, with the key each rides on.
pub const EXPERIMENTAL_KINDS: &[&str] = &["ui.replacements"];

/// The stable kinds (permanent API), by manifest key.
pub const STABLE_KINDS: &[&str] = &[
    "models",
    "measures",
    "metrics",
    "dimensions",
    "collectors",
    "commands",
    "lenses",
    "pages",
    "panels",
    "providers",
    "ui.slots",
    "ui.decorators",
    "ref_kinds",
    "implementations",
    "skills",
    "advisories",
    "event_types",
    "effects",
    "custom_components",
];

impl ManifestV2 {
    /// The experimental kinds this manifest uses.
    pub fn experimental_kinds_used(&self) -> Vec<&'static str> {
        EXPERIMENTAL_KINDS
            .iter()
            .copied()
            .filter(|kind| self.writes(kind))
            .collect()
    }

    /// Whether the manifest writes the experimental kind `kind`. Every
    /// kind in [`EXPERIMENTAL_KINDS`] has its arm — the partition test
    /// writes each and expects each back — so promoting one is moving it
    /// between the tables, nothing more.
    fn writes(&self, kind: &str) -> bool {
        match kind {
            "ui.replacements" => self.ui.replacements.is_some(),
            _ => false,
        }
    }
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
/// top-level `key:` — a mount's lens name inside `slot_mounts`, say.
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
) -> (Vec<T>, Vec<String>) {
    let lines = item_lines(text, kind);
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for (i, item) in items.iter().enumerate() {
        match serde_yaml::from_value(item.clone()) {
            Ok(t) => out.push(t),
            Err(e) => {
                let line = lines.get(i).copied().or(kind_line(text, kind));
                errors.push(at(file, line, format!("`{kind}`: {e}")));
            }
        }
    }
    (out, errors)
}

/// The manifest's intent, its examples and prompts typed one by one: a
/// broken one is left out and is an error at its line.
pub fn intent_of(m: &ManifestV2, file: &str, text: &str) -> (Option<Intent>, Vec<String>) {
    let Some(raw) = &m.intent else {
        return (None, Vec::new());
    };
    let (examples, mut errors) = entries(&raw.examples, "intent.examples", file, text);
    let (prompts, more) = entries(&raw.prompts, "intent.prompts", file, text);
    errors.extend(more);
    let intent = Intent {
        purpose: raw.purpose.clone(),
        origin: raw.origin.clone(),
        examples,
        prompts,
    };
    (Some(intent), errors)
}

/// The manifest's `ui.slots`, typed one by one.
pub fn slots_of(m: &ManifestV2, file: &str, text: &str) -> (Vec<SlotMount>, Vec<String>) {
    entries(&m.ui.slots, "ui.slots", file, text)
}

/// `file:line: message`, or `file: message` when no line is known.
pub fn at(file: &str, line: Option<usize>, message: impl std::fmt::Display) -> String {
    match line {
        Some(n) => format!("{file}:{n}: {message}"),
        None => format!("{file}: {message}"),
    }
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

/// Problems with a manifest's own shape and lifecycle (not its
/// cross-references, which need the loaded lenses). `file` is how the
/// manifest is named in messages; `text` its source, for lines.
pub fn check(m: &ManifestV2, file: &str, text: &str, bundled: bool) -> (Vec<String>, Vec<String>) {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    if m.manifest != CURRENT {
        errors.push(at(
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
        None => errors.push(at(
            file,
            Some(1),
            "`intent` is required: `intent: { purpose: <what it is for>, origin: <thread or effort ref>, examples: [...] }`",
        )),
        Some(intent) => {
            if intent.purpose.trim().is_empty() {
                errors.push(at(file, key_line(text, "intent"), "`intent.purpose` is empty"));
            }
            if let Some(origin) = &intent.origin {
                if oxplow_domain::refs::grammar::CanonicalRef::parse(origin).is_err() {
                    errors.push(at(
                        file,
                        key_line(text, "intent"),
                        format!("`intent.origin` `{origin}` is not a canonical ref (`effort:eff42`, `thread:thr3`)"),
                    ));
                }
            }
            let kinds = oxplow_domain::refs::kind::core_kinds();
            for p in &intent.prompts {
                let line = line_under(text, "intent", &p.prompt).or(key_line(text, "intent"));
                if p.prompt.trim().is_empty() {
                    errors.push(at(file, line, "`intent.prompts`: an empty prompt — write the question"));
                }
                if let Some(problem) = prompt_line_problem(&p.prompt) {
                    errors.push(at(file, line, format!("`intent.prompts`: {problem}")));
                }
                if let Some(about) = &p.about {
                    if kinds.get(about).is_none() {
                        errors.push(at(
                            file,
                            line,
                            format!("`intent.prompts`: `{about}` isn't a kind of ref (`commit`, `file`, `effort`, `work_item`, …)"),
                        ));
                    }
                }
            }
            if intent.examples.is_empty() {
                warnings.push(at(
                    file,
                    key_line(text, "intent"),
                    "`intent.examples` is empty: add at least one input → expected output so the extension can be tested and regenerated",
                ));
            }
        }
    }
    if bundled && m.sharing != Sharing::Shared {
        errors.push(at(
            file,
            key_line(text, "sharing").or(Some(1)),
            "a bundled extension must declare `sharing: shared`",
        ));
    }
    if m.sharing == Sharing::Shared {
        match &m.engine {
            None => errors.push(at(
                file,
                Some(1),
                "a shared extension must declare `engine: \">=MAJOR.MINOR\"` (the oxplow it targets)",
            )),
            Some(engine) => match engine_check(engine, current_engine()) {
                Ok(true) => {}
                Ok(false) => errors.push(at(
                    file,
                    key_line(text, "engine"),
                    format!("needs oxplow {engine}; this is {}", current_engine()),
                )),
                Err(e) => errors.push(at(file, key_line(text, "engine"), e)),
            },
        }
        for kind in m.experimental_kinds_used() {
            errors.push(at(
                file,
                kind_line(text, kind),
                format!(
                    "`{kind}` is experimental: a shared extension may use stable kinds only ({})",
                    STABLE_KINDS.join(", ")
                ),
            ));
        }
    } else if let Some(engine) = &m.engine {
        if let Err(e) = engine_check(engine, current_engine()) {
            errors.push(at(file, key_line(text, "engine"), e));
        }
    }
    (errors, warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> ManifestV2 {
        serde_yaml::from_str(text).unwrap()
    }

    const OK: &str = "manifest: 2\nname: acme\nintent:\n  purpose: Count things\n  examples:\n    - { name: one, input: {}, expect: {} }\n";

    #[test]
    fn a_minimal_private_manifest_is_clean() {
        let m = parse(OK);
        let (errors, warnings) = check(&m, "ext/extension.yaml", OK, false);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(m.sharing, Sharing::Private);
    }

    #[test]
    fn intent_is_required_and_an_empty_examples_list_warns() {
        let text = "manifest: 2\nname: acme\n";
        let (errors, _) = check(&parse(text), "e/extension.yaml", text, false);
        assert!(
            errors.iter().any(|e| e.contains("`intent` is required")),
            "{errors:?}"
        );
        let text = "manifest: 2\nname: acme\nintent:\n  purpose: x\n";
        let (errors, warnings) = check(&parse(text), "e/extension.yaml", text, false);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(warnings.len(), 1);
        assert!(
            warnings[0].starts_with("e/extension.yaml:3:"),
            "{}",
            warnings[0]
        );
    }

    #[test]
    fn shared_needs_engine_and_stable_kinds_only_with_file_line() {
        let text = "manifest: 2\nname: acme\nsharing: shared\nintent:\n  purpose: x\n  examples: [{ name: a }]\nui:\n  replacements: []\n";
        let (errors, _) = check(&parse(text), "e/extension.yaml", text, false);
        assert!(
            errors.iter().any(|e| e.contains("must declare `engine")),
            "{errors:?}"
        );
        assert!(
            errors
                .iter()
                .any(|e| e.contains("`ui.replacements` is experimental")),
            "{errors:?}"
        );
        let text = "manifest: 2\nname: acme\nsharing: shared\nengine: \">=0.1\"\nintent:\n  purpose: x\n  examples: [{ name: a }]\n";
        let (errors, warnings) = check(&parse(text), "e/extension.yaml", text, true);
        assert!(
            errors.is_empty() && warnings.is_empty(),
            "{errors:?} {warnings:?}"
        );
        // Bundled must be shared; a too-new engine is refused.
        let (errors, _) = check(&parse(OK), "e/extension.yaml", OK, true);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("bundled extension must declare `sharing: shared`")),
            "{errors:?}"
        );
        let text = "manifest: 2\nname: acme\nsharing: shared\nengine: \">=99.0\"\nintent:\n  purpose: x\n  examples: [{ name: a }]\n";
        let (errors, _) = check(&parse(text), "e/extension.yaml", text, false);
        assert!(
            errors.iter().any(|e| e.contains("needs oxplow >=99.0")),
            "{errors:?}"
        );
    }

    /// P9.D6: `event_types` is a stable kind — a shared extension may
    /// declare its own event types (oxplow-bundled's verdict is the one
    /// that earned it) — while the kinds still experimental stay closed
    /// to it. Every kind is in exactly one of the two tables.
    #[test]
    fn event_types_is_stable_and_the_tables_partition_the_kinds() {
        let text = "manifest: 2\nname: acme\nsharing: shared\nengine: \">=0.1\"\nintent:\n  purpose: x\n  examples: [{ name: a }]\nevent_types:\n  types: []\n";
        let (errors, _) = check(&parse(text), "e/extension.yaml", text, false);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(STABLE_KINDS.contains(&"event_types"));
        assert!(
            !STABLE_KINDS.iter().any(|k| EXPERIMENTAL_KINDS.contains(k)),
            "a kind is stable or experimental, not both"
        );
        // A manifest using every experimental kind uses exactly those.
        let all = "manifest: 2\nname: acme\nintent: { purpose: x, examples: [{ name: a }] }\nproviders: []\nref_kinds: []\nevent_types: { types: [] }\nui:\n  decorators: []\n  replacements: []\n";
        let mut used = parse(all).experimental_kinds_used();
        used.sort();
        let mut experimental = EXPERIMENTAL_KINDS.to_vec();
        experimental.sort();
        assert_eq!(used, experimental);
        assert!(STABLE_KINDS.contains(&"providers"));
    }

    /// P11 (tsk956): `effects` is stable — oxplow-bundled's follow-up is its
    /// bundled, shared use, approved like any effect — so a shared
    /// extension may declare effects; the tables still partition the kinds.
    #[test]
    fn effects_is_stable_and_the_tables_partition_the_kinds() {
        let text = "manifest: 2\nname: acme\nsharing: shared\nengine: \">=0.1\"\nintent:\n  purpose: x\n  examples: [{ name: a }]\neffects: []\n";
        let (errors, _) = check(&parse(text), "e/extension.yaml", text, false);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(STABLE_KINDS.contains(&"effects"));
        assert!(!EXPERIMENTAL_KINDS.contains(&"effects"));
        assert!(
            !STABLE_KINDS.iter().any(|k| EXPERIMENTAL_KINDS.contains(k)),
            "a kind is stable or experimental, not both"
        );
    }

    /// P11 (tsk962): `custom_components` is stable — the github example's
    /// PR lifetimes is its shared use, a component that acts being a
    /// program a person approves — so a shared extension may declare one;
    /// the tables still partition the kinds.
    #[test]
    fn custom_components_is_stable_and_the_tables_partition_the_kinds() {
        let text = "manifest: 2\nname: acme\nsharing: shared\nengine: \">=0.1\"\nintent:\n  purpose: x\n  examples: [{ name: a }]\ncustom_components: []\n";
        let (errors, _) = check(&parse(text), "e/extension.yaml", text, false);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(STABLE_KINDS.contains(&"custom_components"));
        assert!(!EXPERIMENTAL_KINDS.contains(&"custom_components"));
        assert!(
            !STABLE_KINDS.iter().any(|k| EXPERIMENTAL_KINDS.contains(k)),
            "a kind is stable or experimental, not both"
        );
    }

    /// P10 (K1): `ui.decorators` is stable — oxplow-bundled's verdict
    /// chip is its bundled, shared use — so a shared extension may declare
    /// decorators; the tables still partition the kinds.
    #[test]
    fn decorators_is_stable_and_the_tables_partition_the_kinds() {
        let text = "manifest: 2\nname: acme\nsharing: shared\nengine: \">=0.1\"\nintent:\n  purpose: x\n  examples: [{ name: a }]\nui:\n  decorators: []\n";
        let (errors, _) = check(&parse(text), "e/extension.yaml", text, false);
        assert!(errors.is_empty(), "{errors:?}");
        assert!(STABLE_KINDS.contains(&"ui.decorators"));
        assert!(!EXPERIMENTAL_KINDS.contains(&"ui.decorators"));
        assert!(
            !STABLE_KINDS.iter().any(|k| EXPERIMENTAL_KINDS.contains(k)),
            "a kind is stable or experimental, not both"
        );
    }

    /// Each list item's own line, under a top-level key or under `ui:`,
    /// nested lists and other keys aside.
    #[test]
    fn each_items_line_is_found() {
        let text = "manifest: 2\ncommands:\n  - name: a\n    examples:\n      - { name: x }\n  # gone\n  - name: b\nui:\n  slots: []\n  decorators:\n    - { model: m }\n    - { model: n }\neffects:\n- id: e\n";
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
        let (errors, _) = check(&parse(text), "e/extension.yaml", text, false);
        assert!(
            errors[0].contains("manifest version 3 is not supported"),
            "{errors:?}"
        );
        let text = "manifest: 2\nname: acme\nintent: { purpose: x, origin: nope, examples: [{ name: a }] }\n";
        let (errors, _) = check(&parse(text), "e/extension.yaml", text, false);
        assert!(errors[0].contains("not a canonical ref"), "{errors:?}");
    }

    /// P6.D2: `intent.prompts` are questions the extension helps answer,
    /// each optionally about a kind of ref (what page offers it).
    #[test]
    fn intent_prompts_are_questions_about_a_ref_kind() {
        let text = "manifest: 2\nname: acme\nintent:\n  purpose: x\n  examples: [{ name: a }]\n  prompts:\n    - { prompt: 'Which PRs are waiting on me?' }\n    - { prompt: 'Who reviews this commit?', about: commit }\n";
        let m = parse(text);
        let (errors, _) = check(&m, "e/extension.yaml", text, false);
        assert!(errors.is_empty(), "{errors:?}");
        let prompts = intent_of(&m, "e/extension.yaml", text).0.unwrap().prompts;
        assert_eq!(
            prompts[1],
            IntentPrompt {
                prompt: "Who reviews this commit?".into(),
                about: Some("commit".into())
            }
        );

        for (entry, want) in [
            ("{ prompt: '  ' }", "an empty prompt"),
            // A line break pasted into a terminal is Enter: it would send.
            ("{ prompt: \"Why?\\nAnd how?\" }", "one line"),
            ("{ prompt: x, about: nope }", "`nope` isn't a kind of ref"),
        ] {
            let text = format!("manifest: 2\nname: acme\nintent:\n  purpose: x\n  examples: [{{ name: a }}]\n  prompts:\n    - {entry}\n");
            let (errors, _) = check(&parse(&text), "e/extension.yaml", &text, false);
            assert!(
                errors
                    .iter()
                    .any(|e| e.contains(want) && e.starts_with("e/extension.yaml:")),
                "{entry}: {errors:?}"
            );
        }
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
        let text = "manifest: 2\nname: a\nslot_mounts:\n  - { slot: rail, lens: x }\n  - { slot: commit, lens: y }\nadvisories: []\n";
        assert_eq!(key_line(text, "slot_mounts"), Some(3));
        assert_eq!(line_under(text, "slot_mounts", "lens: y"), Some(5));
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
