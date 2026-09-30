//! `extension.yaml`, manifest version 2 (`.context/extensions.md`;
//! `.context/target-architecture.md` §10).
//!
//! A v2 manifest declares every contribution as data, carries the
//! plugin's **intent**, says whether it is **private** or **shared**, and
//! names the kinds it uses; unknown keys are errors. Kinds have a
//! lifecycle: the stable ones are permanent API, the experimental ones
//! may appear only in a private extension. A v1 manifest (no `manifest:`
//! key) is still read — converted in memory with a warning — so nothing
//! breaks while `oxplow plugin migrate` rewrites the file.
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
}

/// A slot mount: a lens into a core page.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotMount {
    pub slot: String,
    pub lens: String,
}

/// A launcher entry for a target that is not a lens (a page, a command).
/// Parsed and checked for shape in P1; rendered when pages land.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LauncherEntry {
    pub label: String,
    pub category: super::LauncherCategory,
    /// A canonical ref (`page:…`, `command:…`).
    pub target: String,
}

/// `extension.yaml` at `manifest: 2`, as written on disk.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManifestV2 {
    pub manifest: u32,
    pub name: String,
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
    pub gauges: Option<Value>,
    #[serde(default)]
    pub dimensions: Option<Value>,
    /// Data collectors (v1 `sources`): exec / starlark / jaq programs
    /// that produce entities. Parsed by `extension_sources`.
    #[serde(default)]
    pub collectors: Option<Value>,
    /// Starlark command handlers (§7). Parsed as data in P1.
    #[serde(default)]
    pub commands: Option<Value>,
    /// Pages and left-nav panels (§11.3). Parsed as data in P1.
    #[serde(default)]
    pub pages: Option<Value>,
    #[serde(default)]
    pub panels: Option<Value>,
    /// Launcher entries for non-lens targets; a lens lists itself with
    /// its own `launcher:` block.
    #[serde(default)]
    pub launcher: Vec<LauncherEntry>,
    /// Lenses mounted into core pages (v1 `slots`).
    #[serde(default)]
    pub slot_mounts: Vec<SlotMount>,
    /// The instance config schema (§10.3). Parsed as data in P1.
    #[serde(default)]
    pub config: Option<Value>,
    /// Guidance queries for the agent; parsed one by one by the loader.
    /// Stable: the bundled `oxplow-analytics` ships on it, which is the
    /// evidence a kind needs to be promoted.
    #[serde(default)]
    pub advisories: Vec<Value>,

    // ---- experimental kinds (private extensions only) ----
    #[serde(default)]
    pub providers: Option<Value>,
    #[serde(default)]
    pub effects: Option<Value>,
    #[serde(default)]
    pub event_types: Option<Value>,
    #[serde(default)]
    pub ref_kinds: Option<Value>,
    #[serde(default)]
    pub custom_components: Option<Value>,
    #[serde(default)]
    pub decorators: Option<Value>,
    #[serde(default)]
    pub replacements: Option<Value>,
}

/// The kinds a shared extension may not use, with the key each rides on.
pub const EXPERIMENTAL_KINDS: &[&str] = &[
    "providers",
    "effects",
    "event_types",
    "ref_kinds",
    "custom_components",
    "decorators",
    "replacements",
];

/// The stable kinds (permanent API), by manifest key.
pub const STABLE_KINDS: &[&str] = &[
    "models",
    "measures",
    "metrics",
    "gauges",
    "dimensions",
    "collectors",
    "commands",
    "lenses",
    "pages",
    "panels",
    "launcher",
    "slot_mounts",
    "config",
    "advisories",
];

impl ManifestV2 {
    /// The experimental kinds this manifest uses.
    pub fn experimental_kinds_used(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        let present = |v: &Option<Value>| v.is_some();
        if present(&self.providers) {
            out.push("providers");
        }
        if present(&self.effects) {
            out.push("effects");
        }
        if present(&self.event_types) {
            out.push("event_types");
        }
        if present(&self.ref_kinds) {
            out.push("ref_kinds");
        }
        if present(&self.custom_components) {
            out.push("custom_components");
        }
        if present(&self.decorators) {
            out.push("decorators");
        }
        if present(&self.replacements) {
            out.push("replacements");
        }
        out
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
                key_line(text, kind),
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
    for entry in &m.launcher {
        if oxplow_domain::refs::grammar::CanonicalRef::parse(&entry.target).is_err() {
            errors.push(at(
                file,
                line_under(text, "launcher", &entry.target),
                format!(
                    "launcher entry `{}` targets `{}`, which is not a canonical ref",
                    entry.label, entry.target
                ),
            ));
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
        let text = "manifest: 2\nname: acme\nsharing: shared\nintent:\n  purpose: x\n  examples: [{ name: a }]\nref_kinds:\n  - kind: ticket\n";
        let (errors, _) = check(&parse(text), "e/extension.yaml", text, false);
        assert!(
            errors.iter().any(|e| e.contains("must declare `engine")),
            "{errors:?}"
        );
        let ref_kinds = errors
            .iter()
            .find(|e| e.contains("`ref_kinds` is experimental"))
            .unwrap();
        assert!(ref_kinds.starts_with("e/extension.yaml:7:"), "{ref_kinds}");
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
}
