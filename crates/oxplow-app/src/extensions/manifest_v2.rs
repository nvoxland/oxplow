//! `extension.yaml`, manifest version 2 (`.context/extensions.md`).
//!
//! A v2 manifest declares every contribution as data, carries the
//! plugin's **intent**, says whether it is **private** or **shared**, and
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, Default)]
#[serde(rename_all = "lowercase")]
pub enum Sharing {
    #[default]
    Private,
    Shared,
}

/// One acceptance example: an input and what the extension should make
/// of it. Fixtures for `oxplow plugin test`; data here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
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

/// A question an extension helps answer (P6.D2). `about` is a ref kind
/// (`commit`, `file`, `effort`): a page for a ref of that kind suggests it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct IntentPrompt {
    pub prompt: String,
    #[serde(default)]
    pub about: Option<String>,
}

/// A slot mount: a lens into a core page.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotMount {
    pub slot: String,
    pub lens: String,
}

/// `ui:` — everything an extension adds to the core UI (P6b): lenses
/// mounted into core pages (`slots`, stable), decorations on core refs
/// (`decorators`, stable since P10) and replaced sub-components
/// (`replacements`, experimental; `extensions/replacements.rs`). Its
/// commands meet a person through their own `ui` (`commands:`).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiBlock {
    #[serde(default)]
    pub slots: Vec<SlotMount>,
    #[serde(default)]
    pub decorators: Option<Value>,
    #[serde(default)]
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
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestV2 {
    pub manifest: u32,
    pub name: String,
    /// The namespace its commands' ids are under
    /// (`<namespace>.<area>.<verb>`); defaults to the name with `-` → `_`.
    /// `oxplow` is reserved for the extensions that ship with oxplow.
    #[serde(default)]
    pub namespace: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub sharing: Sharing,
    /// The oxplow version range it targets, `>=MAJOR.MINOR[.PATCH]`.
    /// Required when shared.
    #[serde(default)]
    pub engine: Option<String>,
    pub intent: Option<Intent>,

    // ---- stable kinds ----
    /// SQL models (§9.1): `ModelDecl`s, each with `models/<name>.sql`,
    /// published as `v_<extension>_<name>` (P4.9).
    #[serde(default)]
    pub models: Option<Value>,
    /// The metric catalog contributions, in `.oxplow/project.yaml`'s
    /// vocabulary; validated per block by the loader.
    #[serde(default)]
    pub measures: Option<Value>,
    #[serde(default)]
    pub metrics: Option<Value>,
    #[serde(default)]
    pub dimensions: Option<Value>,
    /// Collectors (P7.B3): exec / starlark /
    /// jaq / read, writing entities or recording facts. Parsed by
    /// `oxplow_config::collectors`.
    #[serde(default)]
    pub collectors: Option<Value>,
    /// Commands whose handler is a Starlark script composing core commands
    /// (§7, P6b). Parsed by `extension_commands`.
    #[serde(default)]
    pub commands: Option<Value>,
    /// Pages and left-nav panels (§11.3). Parsed as data in P1.
    #[serde(default)]
    pub pages: Option<Value>,
    #[serde(default)]
    pub panels: Option<Value>,
    /// What it adds to the core UI: slots, commands in menus, decorators.
    #[serde(default)]
    pub ui: UiBlock,
    /// The instance config schema (§10.3). Parsed as data in P1.
    #[serde(default)]
    pub config: Option<Value>,
    /// Guidance queries for the agent; parsed one by one by the loader.
    /// Stable: bundled `oxplow-bundled` ships on it, which is the
    /// evidence a kind needs to be promoted.
    #[serde(default)]
    pub advisories: Vec<Value>,
    /// The event types it may log (P9.D6). Stable.
    #[serde(default)]
    pub event_types: Option<Value>,

    /// Kinds of thing a ref can name (stable since P10).
    #[serde(default)]
    pub ref_kinds: Option<Value>,
    /// Capability implementations: built-ins of core's it declares
    /// (stable; `oxplow-bundled` declares the defaults).
    #[serde(default)]
    pub implementations: Option<Value>,
    /// Skills and slash commands for the coding agent (stable).
    #[serde(default)]
    pub skills: Option<Value>,

    /// External providers: programs implementing a capability, whose
    /// operations its `commands:` declare (stable).
    #[serde(default)]
    pub providers: Option<Value>,
    #[serde(default)]
    pub effects: Option<Value>,
    #[serde(default)]
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
    "config",
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

/// The line of a kind's key: a top-level key, or `ui.<key>` under `ui:`.
pub fn kind_line(text: &str, kind: &str) -> Option<usize> {
    match kind.strip_prefix("ui.") {
        Some(sub) => line_under(text, "ui", &format!("{sub}:")).or(key_line(text, "ui")),
        None => key_line(text, kind),
    }
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
    match &m.intent {
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

    #[test]
    fn unknown_keys_and_bad_versions_are_errors() {
        assert!(serde_yaml::from_str::<ManifestV2>("manifest: 2\nname: a\nslots: []\n").is_err());
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
        let prompts = &m.intent.as_ref().unwrap().prompts;
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
