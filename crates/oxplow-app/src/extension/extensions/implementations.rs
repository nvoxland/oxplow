//! `implementations:` in `extension.yaml` (stable): the capability
//! implementations an extension declares
//! (`.context/extensions-commands.md` "Implementations").
//!
//! ```yaml
//! implementations:
//!   - capability: snapshots
//!     id: hashes
//!     entry: oxplow:snapshot-hashes
//! ```
//!
//! An entry names a built-in in core's standard library
//! (`capabilities::BUILT_INS`), the way a collector names `oxplow:junit`;
//! which one is active is the person's and the project's choice
//! (`capabilities::CapabilityRegistry::resolve`) — or, for a capability
//! many implementations serve (`agent_harness`, `ai_provider`), every one
//! declared. `config:` configures a built-in that takes one, checked
//! against its schema (none does today). An `ai_provider` names no entry:
//! `provider:` is the Rust kind it is (`oxplow_ai_rig::KINDS`).
//!
//! An entry that is a path in the extension is a **script implementation**
//! of a capability whose interface is a set of verbs (its provider contract,
//! `oxplow_domain::capability::ProviderContract`): the script defines one
//! function per verb, which core calls through the same dispatch as an
//! external provider's (`providers::script`). It declares what a provider
//! declares of itself — `features:`, `data:`, an item-keeping capability's `stages:`,
//! `fields:` and `id_pattern:` — and the scopes it calls in `needs:`.

use oxplow_domain::diagnostic::Diagnostic;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use super::manifest_v2::{at, item_lines, key_line};

/// One implementation as the extension declares it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ImplementationDecl {
    pub capability: String,
    /// What `activeProviders` names.
    pub id: String,
    /// How a person names it; the built-in's own title when absent.
    pub title: Option<String>,
    /// The built-in it is (`oxplow:snapshot-hashes`); its features are core's to
    /// say. `None` for an AI provider, which names its kind instead.
    pub entry: Option<String>,
    /// An AI provider's Rust kind (`anthropic`, `openai_compatible`,
    /// `typesafe`, …: `oxplow_ai_rig::KINDS`); `None` for anything else.
    pub provider: Option<String>,
    /// The extension's skills (`skills:`) it owns: offered only while it's
    /// the active implementation.
    pub skills: Vec<String>,
    /// What the declaration configures, checked against the built-in's
    /// schema; `{}` when it says nothing.
    #[specta(type = specta_typescript::Any)]
    pub config: serde_json::Value,
    /// Its logo, as a `data:image/svg+xml;base64,…` URI for an `<img>`
    /// (never markup): the SVG `icon:` names inside the extension, checked
    /// by [`load_icon`].
    pub icon: Option<String>,
    /// One sentence saying what this implementation is
    /// (`v_capability_provider.description`).
    pub description: Option<String>,
    /// The scopes a script implementation calls (`sql.read`); empty for
    /// anything else.
    pub needs: Vec<String>,
    /// A script implementation: its text and what it declares, when `entry`
    /// names a file of the extension rather than a built-in.
    #[serde(skip)]
    #[specta(skip)]
    pub script: Option<ScriptImplementation>,
    /// The setting that offers it (`when:`); `None`: always offered.
    pub when: Option<super::config_schema::ConfigCondition>,
}

/// A capability implementation written as a script (`entry:` a Starlark
/// file of the extension): what it declares, in the shape an external
/// provider declares it, so the capability's host checks and adapts it the
/// same way (`providers::script`).
#[derive(Debug, Clone, PartialEq)]
pub struct ScriptImplementation {
    /// The script's text.
    pub text: String,
    /// Its declarations as a provider's: the capability with its
    /// `features` and `data`, and one verb (`confirm: never`, `access:
    /// record`) for each of the contract's verbs the script defines.
    pub declared: oxplow_provider_protocol::model::InitializeResult,
    /// Its spec as a provider's: its id, capability and entry, a task
    /// tracker's `stages`, `fields` and `id_pattern`, and its `needs`.
    pub spec: crate::providers::spec::ProviderSpec,
}

// Its declarations hold strings and JSON values only, which compare totally.
impl Eq for ScriptImplementation {}

/// An `implementations:` entry as the manifest holds it.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImplementationFile {
    capability: String,
    id: String,
    #[serde(default)]
    title: Option<String>,
    /// The built-in it is (`oxplow:snapshot-hashes`), or a script in the extension
    /// that implements the capability's verbs.
    #[serde(default)]
    entry: Option<String>,
    /// An `ai_provider`'s Rust kind (`anthropic`, `openai`,
    /// `openai_compatible`, `openrouter`, `ollama`, `typesafe`, …), in place
    /// of `entry`.
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    skills: Vec<String>,
    /// What it configures, checked against the built-in's schema.
    #[serde(default)]
    #[schemars(with = "Option<serde_json::Value>")]
    config: Option<Value>,
    /// A path to an SVG in the extension (`icons/claude.svg`): its logo,
    /// shown wherever the implementation is (a session's tab). Any
    /// capability may declare one.
    #[serde(default)]
    icon: Option<String>,
    /// One sentence saying what it is.
    #[serde(default)]
    description: Option<String>,
    /// The scopes a script implementation calls (`sql.read`): what a
    /// composing script can, and the tree (`tree.read`) and its own private
    /// state (`state.write`) too (`scope_calls::check_implementation_need`).
    #[serde(default)]
    needs: Vec<String>,
    /// A script implementation's features (`{ links: true }`): the
    /// capability's, each true or false.
    #[serde(default)]
    #[schemars(with = "Option<serde_json::Value>")]
    features: Option<Value>,
    /// What a script implementation declares of itself beyond its features:
    /// its capability's `data` (an agent harness's environment markers and
    /// settings).
    #[serde(default)]
    #[schemars(with = "Option<serde_json::Value>")]
    data: Option<Value>,
    /// Offered only while a setting of the extension's own configuration
    /// is one of `in` (`{ config: /store, in: [true] }`); otherwise it is
    /// declared but off.
    #[serde(default)]
    when: Option<super::config_schema::ConfigCondition>,
}

impl ImplementationFile {
    /// The first key only a script implementation takes that this entry
    /// sets: a built-in's and an AI provider's are core's to say.
    fn script_key(&self) -> Option<&'static str> {
        [
            ("needs", !self.needs.is_empty()),
            ("features", self.features.is_some()),
            ("data", self.data.is_some()),
        ]
        .into_iter()
        .find_map(|(key, set)| set.then_some(key))
    }
}

/// The largest icon file.
const ICON_MAX_BYTES: usize = 16 * 1024;

/// The SVG at `path` in the extension as a data URI. It is only ever
/// drawn as an image, so a script in it would not run; one is refused all
/// the same, as is anything but a small `.svg` inside the extension.
/// A problem not yet placed in a file: `.at(file, line)` places it.
fn problem(code: &str, message: String) -> Box<Diagnostic> {
    Box::new(Diagnostic::error(code, message))
}

fn load_icon(path: &str, read: &dyn Fn(&str) -> Option<String>) -> Result<String, Box<Diagnostic>> {
    if path.starts_with('/') || path.split('/').any(|p| p == "..") {
        return Err(problem(
            "implementation.icon_outside",
            format!("icon `{path}` must be inside the extension"),
        ));
    }
    if !path.to_ascii_lowercase().ends_with(".svg") {
        return Err(problem(
            "implementation.icon_not_svg",
            format!("icon `{path}` must be an `.svg` file"),
        ));
    }
    let Some(text) = read(path) else {
        return Err(problem(
            "implementation.icon_missing",
            format!("icon `{path}` isn't in the extension"),
        ));
    };
    if text.len() > ICON_MAX_BYTES {
        return Err(problem(
            "implementation.icon_too_big",
            format!(
                "icon `{path}` is too big ({} bytes; at most {ICON_MAX_BYTES})",
                text.len()
            ),
        ));
    }
    let body = text.trim_start();
    let body = body
        .strip_prefix("<?xml")
        .and_then(|rest| rest.split_once("?>"))
        .map_or(body, |(_, rest)| rest.trim_start());
    if !body.starts_with("<svg") {
        return Err(problem(
            "implementation.icon_invalid",
            format!("icon `{path}` isn't an SVG (it must start with `<svg`)"),
        ));
    }
    if text.to_ascii_lowercase().contains("<script") {
        return Err(problem(
            "implementation.icon_script",
            format!("icon `{path}` carries a script; an icon is only a picture"),
        ));
    }
    use base64::Engine as _;
    Ok(format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(text.as_bytes())
    ))
}

/// Parse `implementations:`: the valid declarations, and what's wrong
/// with the rest (at their lines).
pub fn parse_implementations(
    value: &Value,
    file: &str,
    manifest: &str,
    read: &dyn Fn(&str) -> Option<String>,
    configuration: Option<&super::config_schema::ConfigurationFile>,
) -> (Vec<ImplementationDecl>, Vec<Diagnostic>) {
    let block = key_line(manifest, "implementations");
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(
                "implementation.not_a_list",
                file,
                block,
                "`implementations` must be a list",
            )],
        );
    };
    let mut out: Vec<ImplementationDecl> = Vec::new();
    let mut errors = Vec::new();
    let lines = item_lines(manifest, "implementations");
    for (i, item) in items.iter().enumerate() {
        let item_line = lines.get(i).copied().or(block);
        let f: ImplementationFile = match serde_yaml::from_value(item.clone()) {
            Ok(f) => f,
            Err(e) => {
                errors.push(at(
                    "implementation.invalid",
                    file,
                    item_line,
                    format!("implementation: {e}"),
                ));
                continue;
            }
        };
        let line = item_line;
        if let Some((code, why)) = f.when.as_ref().and_then(|w| w.problem(configuration)) {
            errors.push(at(
                code,
                file,
                line,
                format!("`{}` implementation `{}`: {why}", f.capability, f.id),
            ));
            continue;
        }
        match decl_of(f, read) {
            Ok(d)
                if out
                    .iter()
                    .any(|o| o.capability == d.capability && o.id == d.id) =>
            {
                errors.push(at(
                    "implementation.duplicate",
                    file,
                    line,
                    format!(
                        "`{}` implementation `{}` is declared twice",
                        d.capability, d.id
                    ),
                ))
            }
            Ok(d) => out.push(d),
            Err(e) => errors.push(e.at(file, line)),
        }
    }
    (out, errors)
}

fn decl_of(
    f: ImplementationFile,
    read: &dyn Fn(&str) -> Option<String>,
) -> Result<ImplementationDecl, Box<Diagnostic>> {
    use oxplow_domain::capability;
    let Some(spec) = capability::spec(&f.capability).filter(|c| c.choosable || c.many) else {
        return Err(problem(
            "implementation.unknown_capability",
            format!(
                "`{}` isn't a capability an extension implements ({})",
                f.capability,
                capability::CAPABILITIES
                    .iter()
                    .filter(|c| c.choosable || c.many)
                    .map(|c| c.id)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    };
    // A required capability's default is core's own; a many-capability's
    // default is just the one used when nothing names one.
    if !spec.optional && !spec.many && f.id == spec.default {
        return Err(problem(
            "implementation.core_default",
            format!(
                "`{}` implementation `{}` is core's own (a required capability's default); \
             declare another id",
                spec.id, f.id
            ),
        ));
    }
    if f.id == capability::NONE
        || f.id.is_empty()
        || !f
            .id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(problem(
            "implementation.invalid_id",
            format!(
                "implementation id `{}` must be lowercase letters, digits and `_`, and not `none`",
                f.id
            ),
        ));
    }
    let icon = f.icon.as_deref().map(|p| load_icon(p, read)).transpose()?;
    if f.capability == "ai_provider" {
        return provider_decl(f, icon);
    }
    if f.provider.is_some() {
        return Err(problem(
            "implementation.provider_on_entry",
            format!(
                "`provider:` names an AI provider's kind; a `{}` implementation names an `entry:`",
                f.capability
            ),
        ));
    }
    let Some(entry) = f.entry.clone() else {
        return Err(problem(
            "implementation.missing_entry",
            format!(
                "`{}` implementation `{}` needs an `entry:`",
                f.capability, f.id
            ),
        ));
    };
    if !entry.starts_with("oxplow:") {
        return script_decl(f, &entry, spec, read, icon);
    }
    if let Some(key) = f.script_key() {
        let code = if key == "needs" {
            "implementation.built_in_needs"
        } else {
            "implementation.built_in_declares"
        };
        return Err(problem(
            code,
            format!(
                "entry `{entry}` takes no `{key}`: a built-in is core's code, and core says its \
                 {key} (a script implementation declares them)"
            ),
        ));
    }
    let Some(built_in) = crate::capabilities::built_in(&entry) else {
        return Err(problem(
            "implementation.unknown_built_in",
            format!(
                "entry `{entry}` isn't one of core's built-ins ({})",
                crate::capabilities::BUILT_INS
                    .iter()
                    .map(|b| b.entry)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    };
    if built_in.capability != spec.id {
        return Err(problem(
            "implementation.built_in_capability",
            format!(
                "entry `{entry}` implements `{}`, not `{}`",
                built_in.capability, spec.id
            ),
        ));
    }
    if let Some(provider) = built_in.provider.filter(|p| *p != f.id) {
        return Err(problem("implementation.built_in_provider", format!(
            "entry `{entry}` is declared under `{provider}`, the provider its items' refs carry, not `{}`",
            f.id
        )));
    }
    let config = match f.config {
        Some(v) => serde_json::to_value(v)
            .map_err(|e| problem("implementation.invalid_config", format!("config: {e}")))?,
        None => serde_json::json!({}),
    };
    match built_in.config_schema {
        Some(schema) => capability::check_config(schema, &config).map_err(|e| {
            problem(
                "implementation.invalid_config",
                format!("`{}` implementation `{}`: {e}", spec.id, f.id),
            )
        })?,
        None if config != serde_json::json!({}) => {
            return Err(problem(
                "implementation.built_in_config",
                format!("entry `{entry}` takes no config"),
            ))
        }
        None => {}
    }
    Ok(ImplementationDecl {
        capability: f.capability,
        id: f.id,
        title: f.title,
        entry: Some(entry),
        provider: None,
        skills: f.skills,
        config,
        icon,
        description: f.description,
        needs: Vec::new(),
        script: None,
        when: f.when,
    })
}

/// A script implementation of a capability whose interface is a set of
/// verbs (its provider contract): checked as it will be called — the
/// script parses, uses only the scopes in its `needs`, and defines every
/// verb the contract requires for the features it declares; what it
/// declares of itself meets the rules an external provider's declarations
/// do (`spec::check_declarations`, `spec::items_problem`).
fn script_decl(
    f: ImplementationFile,
    entry: &str,
    spec: &oxplow_domain::capability::CapabilitySpec,
    read: &dyn Fn(&str) -> Option<String>,
    icon: Option<String>,
) -> Result<ImplementationDecl, Box<Diagnostic>> {
    let what = format!("`{}` implementation `{}`", f.capability, f.id);
    let contract = script_contract(spec, entry, &what)?;
    // Its id is the segment of its items' refs, as an instance's is.
    if let Some(why) = oxplow_domain::capability::instance_id_problem(&f.id) {
        return Err(problem(
            "implementation.script_id",
            format!("{what}: a script implementation's id {why}"),
        ));
    }
    if entry.starts_with('/') || entry.split('/').any(|p| p == ".." || p.is_empty()) {
        return Err(problem(
            "implementation.script_outside",
            format!("{what}: entry `{entry}` must be a file inside the extension"),
        ));
    }
    let to_json = |key: &str, v: &Value| {
        serde_json::to_value(v).map_err(|e| {
            problem(
                "implementation.script_declares",
                format!("{what}: {key}: {e}"),
            )
        })
    };
    // Its config is handed to every verb (`x["config"]`), as an instance's
    // config is to its process: any map.
    let config = match &f.config {
        Some(v) => to_json("config", v)?,
        None => serde_json::json!({}),
    };
    if !config.is_object() {
        return Err(problem(
            "implementation.script_config",
            format!("{what}: `config` must be a map (each verb gets it as `x[\"config\"]`)"),
        ));
    }
    for need in &f.needs {
        crate::scope_calls::check_implementation_need(need).map_err(|e| {
            problem(
                "implementation.script_need",
                format!("{what}: needs `{need}`: {e}"),
            )
        })?;
    }
    let text = read(entry).ok_or_else(|| {
        problem(
            "implementation.script_missing",
            format!("{what}: entry `{entry}` isn't in the extension"),
        )
    })?;
    let defined = oxplow_script::runtime::top_level_defs(entry, &text)
        .and_then(|defined| {
            crate::services::check_uses(
                &text,
                &f.needs,
                crate::scope_calls::check_implementation_need,
            )
            .map(|()| defined)
        })
        .map_err(|e| {
            problem(
                "implementation.script_invalid",
                format!("{what}: `{entry}` {e}"),
            )
        })?;
    let mut features = match &f.features {
        Some(v) => to_json("features", v)?,
        None => serde_json::json!({}),
    };
    // Every feature of the capability is said, on or off, as a provider's
    // declarations say them: one it leaves out is off.
    if let Some(declared) = features.as_object_mut() {
        for feature in spec.features {
            declared
                .entry(feature.to_string())
                .or_insert(serde_json::Value::Bool(false));
        }
    }
    let data = match &f.data {
        Some(v) => to_json("data", v)?,
        None => serde_json::Value::Null,
    };
    let provider_spec = crate::providers::spec::ProviderSpec {
        id: f.id.clone(),
        capability: f.capability.clone(),
        entry: Some(entry.to_string()),
        args: Vec::new(),
        env: Vec::new(),
        credentials: Vec::new(),
        network: Vec::new(),
        declarations: entry.to_string(),
        needs: f.needs.clone(),
    };
    for verb in contract.required(&features) {
        if !defined.iter().any(|d| d == verb) {
            let when = match contract.verb(verb).map(|v| v.needs) {
                Some(oxplow_domain::capability::Need::Feature(feature)) => {
                    format!(" when it declares `{feature}`")
                }
                _ => String::new(),
            };
            return Err(problem(
                "implementation.script_missing_verb",
                format!(
                    "{what}: `{entry}` defines no `{verb}` (`def {verb}(x):`), a verb a `{}` \
                     implementation answers{when}",
                    spec.id
                ),
            ));
        }
    }
    use oxplow_provider_protocol::model as wire;
    let declared = wire::InitializeResult {
        protocol_version: wire::PROTOCOL_VERSION.to_string(),
        provider: wire::Party {
            name: f.title.clone().unwrap_or_else(|| f.id.clone()),
            version: "script".into(),
        },
        capabilities: vec![wire::CapabilityDecl {
            capability: f.capability.clone(),
            features,
            data,
        }],
        // Core checks a call against the capability's own command
        // (`task.<verb>`) before it reaches the script: any object here.
        commands: contract
            .verbs
            .iter()
            .filter(|v| defined.iter().any(|d| d == v.name))
            .map(|v| wire::CommandDecl {
                name: v.name.to_string(),
                summary: format!("`{entry}`'s `{}`.", v.name),
                input_schema: serde_json::json!({ "type": "object" }),
                confirm: "never".into(),
                access: "record".into(),
                undoable: false,
            })
            .collect(),
        event_types: Vec::new(),
        collectors: Vec::new(),
        config_schema: serde_json::json!({ "type": "object" }),
    };
    crate::providers::spec::check_declarations(&provider_spec, &declared).map_err(|e| {
        let prefix = format!("provider `{}`: ", f.id);
        problem(
            "implementation.script_declares",
            format!("{what}: {}", e.strip_prefix(&prefix).unwrap_or(&e)),
        )
    })?;
    Ok(ImplementationDecl {
        capability: f.capability,
        id: f.id,
        title: f.title,
        entry: Some(entry.to_string()),
        provider: None,
        skills: f.skills,
        config,
        icon,
        description: f.description,
        needs: f.needs,
        script: Some(ScriptImplementation {
            text,
            declared,
            spec: provider_spec,
        }),
        when: f.when,
    })
}

/// The verbs a script implementation of `spec` answers: its provider
/// contract, or why a script can't implement it (a capability with none).
fn script_contract(
    spec: &oxplow_domain::capability::CapabilitySpec,
    entry: &str,
    what: &str,
) -> Result<&'static oxplow_domain::capability::ProviderContract, Box<Diagnostic>> {
    spec.provider.ok_or_else(|| {
        problem(
            "implementation.script_no_verbs",
            format!(
                "{what}: entry `{entry}` is a script, and a script implements a capability \
                 through its verbs — `{}` has none (name one of core's built-ins: `oxplow:<name>`)",
                spec.id
            ),
        )
    })
}

/// An AI provider: the Rust kind it is (`provider:`, one of
/// `oxplow_ai_rig::KINDS`) and its `config: { baseUrl? }`, the API base
/// an instance that names none uses.
fn provider_decl(
    f: ImplementationFile,
    icon: Option<String>,
) -> Result<ImplementationDecl, Box<Diagnostic>> {
    let known = || oxplow_ai_rig::names().join(", ");
    if let Some(key) = f.script_key() {
        let code = if key == "needs" {
            "implementation.provider_needs"
        } else {
            "implementation.provider_declares"
        };
        return Err(problem(
            code,
            format!(
                "`ai_provider` implementation `{}` takes no `{key}` (it is a Rust kind, and \
                 calls nothing of oxplow's)",
                f.id
            ),
        ));
    }
    if f.entry.is_some() {
        return Err(problem(
            "implementation.provider_entry",
            format!(
            "`ai_provider` implementation `{}` names its kind with `provider:` ({}), not `entry:`",
            f.id,
            known()
        ),
        ));
    }
    let Some(kind) = f.provider else {
        return Err(problem(
            "implementation.provider_missing_kind",
            format!(
                "`ai_provider` implementation `{}` needs `provider:`, the kind it is ({})",
                f.id,
                known()
            ),
        ));
    };
    oxplow_ai_rig::kind(&kind).map_err(|e| {
        problem(
            "implementation.invalid_config",
            format!("`ai_provider` implementation `{}`: {e}", f.id),
        )
    })?;
    let config = match f.config {
        Some(v) => serde_json::to_value(v)
            .map_err(|e| problem("implementation.invalid_config", format!("config: {e}")))?,
        None => serde_json::json!({}),
    };
    let bad = |code: &'static str, what: String| {
        problem(
            code,
            format!("`ai_provider` implementation `{}`: {what}", f.id),
        )
    };
    let Some(fields) = config.as_object() else {
        return Err(bad(
            "implementation.invalid_config",
            "`config` must be a map".into(),
        ));
    };
    for (key, value) in fields {
        match key.as_str() {
            "baseUrl" if value.is_string() => {}
            "baseUrl" => {
                return Err(bad(
                    "implementation.invalid_config",
                    "`config.baseUrl` must be a URL".into(),
                ))
            }
            other => {
                return Err(bad(
                    "implementation.invalid_config",
                    format!("config: unknown key `{other}` (an AI provider takes `baseUrl`)"),
                ))
            }
        }
    }
    Ok(ImplementationDecl {
        capability: f.capability,
        id: f.id,
        title: f.title,
        entry: None,
        provider: Some(kind),
        skills: f.skills,
        config,
        icon,
        description: f.description,
        needs: Vec::new(),
        script: None,
        when: f.when,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> (Vec<ImplementationDecl>, Vec<Diagnostic>) {
        parse_with(yaml, &|_| None)
    }

    fn parse_with(
        yaml: &str,
        read: &dyn Fn(&str) -> Option<String>,
    ) -> (Vec<ImplementationDecl>, Vec<Diagnostic>) {
        let manifest = format!("implementations:\n{yaml}");
        let doc: Value = serde_yaml::from_str(&manifest).unwrap();
        parse_implementations(
            &doc["implementations"],
            "extension.yaml",
            &manifest,
            read,
            None,
        )
    }

    /// A `when:` is checked against the extension's own configuration: one
    /// naming a key it doesn't declare is refused, one naming a declared
    /// boolean is kept.
    #[test]
    fn a_when_is_checked_against_the_extensions_configuration() {
        let configuration: super::super::config_schema::ConfigurationFile = serde_yaml::from_str(
            "version: 1\nschema: { type: object, properties: { store: { type: boolean, default: true } } }\n",
        )
        .unwrap();
        let yaml = |pointer: &str| {
            let manifest = format!(
                "implementations:\n  - {{ capability: snapshots, id: hashes, entry: \"oxplow:snapshot-hashes\", when: {{ config: {pointer}, in: [true] }} }}\n"
            );
            let doc: Value = serde_yaml::from_str(&manifest).unwrap();
            parse_implementations(
                &doc["implementations"],
                "extension.yaml",
                &manifest,
                &|_| None,
                Some(&configuration),
            )
        };
        let (decls, errors) = yaml("/store");
        assert!(errors.is_empty(), "{errors:?}");
        assert!(decls[0].when.is_some());
        let (decls, errors) = yaml("/nope");
        assert!(decls.is_empty());
        assert_eq!(errors[0].code, "when.unknown_key", "{errors:?}");
    }

    /// An AI provider names the Rust kind it is with `provider:` (no
    /// `entry:`) and takes `config: { baseUrl? }`. An unknown
    /// kind is refused, naming the known ones.
    #[test]
    fn an_ai_provider_names_its_kind() {
        let (decls, errors) = parse(
            "  - { capability: ai_provider, id: acme, title: Acme, provider: openai_compatible, config: { baseUrl: \"https://api.acme.test\" } }\n",
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(decls[0].provider.as_deref(), Some("openai_compatible"));
        assert_eq!(decls[0].entry, None);
        assert_eq!(decls[0].config["baseUrl"], "https://api.acme.test");
        let errors = |yaml: &str| {
            parse(yaml)
                .1
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join("\n")
        };
        for (yaml, want) in [
            (
                "  - { capability: ai_provider, id: acme, provider: nope }\n",
                "no AI provider kind `nope` (oxplow's kinds: anthropic,",
            ),
            (
                "  - { capability: ai_provider, id: acme }\n",
                "needs `provider:`",
            ),
            (
                "  - { capability: ai_provider, id: acme, entry: providers/p.star }\n",
                "names its kind with `provider:`",
            ),
            (
                "  - { capability: ai_provider, id: acme, provider: anthropic, config: { ops: [decide] } }\n",
                "unknown key `ops`",
            ),
            (
                "  - { capability: snapshots, id: acme, provider: anthropic, entry: oxplow:snapshots }\n",
                "`provider:` names an AI provider's kind",
            ),
        ] {
            let got = errors(yaml);
            assert!(got.contains(want), "{yaml}: {got}");
        }
    }

    const SNAP: &str = "def mark(x):\n    return {}\n\
        def changed(x):\n    return {\"result\": scope(\"sql.read\", {\"sql\": \"SELECT 1\"})}\n\
        def _helper(x):\n    return x\n";

    fn scripts(rel: &str) -> Option<String> {
        match rel {
            "snaps/s.star" => Some(SNAP.to_string()),
            "snaps/full.star" => Some(format!("{SNAP}def read_at(x):\n    return {{}}\n")),
            "snaps/one.star" => Some("def mark(x):\n    return {}\n".to_string()),
            "snaps/broken.star" => Some("def mark(x)\n".to_string()),
            _ => None,
        }
    }

    /// An entry that is a file of the extension is a script implementing
    /// the capability's verbs: it keeps its `needs`, and what it declares
    /// of itself as an external provider's declarations would say it —
    /// the verbs it defines (helpers aside) and its features.
    #[test]
    fn an_entry_may_be_a_script_implementing_the_verbs() {
        let (decls, errors) = parse_with(
            "  - { capability: snapshots, id: marks, title: Marks, entry: snaps/full.star, needs: [sql.read], features: { contents: true }, config: { prefix: n } }\n",
            &scripts,
        );
        assert!(errors.is_empty(), "{errors:?}");
        let d = &decls[0];
        assert_eq!(d.entry.as_deref(), Some("snaps/full.star"));
        assert_eq!(d.needs, vec!["sql.read".to_string()]);
        assert_eq!(d.config["prefix"], "n");
        let script = d.script.as_ref().unwrap();
        assert_eq!(script.text, scripts("snaps/full.star").unwrap());
        let verbs: Vec<&str> = script
            .declared
            .commands
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(verbs, ["mark", "changed", "read_at"]);
        assert_eq!(
            script.declared.capabilities[0].features,
            serde_json::json!({ "contents": true })
        );
        // A feature it leaves out is said, off.
        let (decls, errors) = parse_with(
            "  - { capability: snapshots, id: marks, entry: snaps/s.star, needs: [sql.read] }\n",
            &scripts,
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            decls[0].script.as_ref().unwrap().declared.capabilities[0].features,
            serde_json::json!({ "contents": false })
        );
    }

    /// What a script implementation can't be is refused at load, each with
    /// its own code: a script that isn't there or doesn't parse, a verb the
    /// contract requires (for the features it declares) left undefined, a
    /// feature or need that isn't one, a scope it calls without needing,
    /// stages on a capability with no items, a config that isn't a map, an
    /// id its refs can't carry; and a built-in or an AI provider is refused
    /// the keys only a script takes.
    #[test]
    fn a_bad_script_implementation_is_refused() {
        let errors = |yaml: &str| {
            parse_with(yaml, &scripts)
                .1
                .iter()
                .map(|d| format!("{}: {}", d.code, d.message))
                .collect::<Vec<_>>()
                .join("\n")
        };
        let snap = |extra: &str| {
            format!("  - {{ capability: snapshots, id: marks, entry: snaps/s.star, needs: [sql.read]{extra} }}\n")
        };
        for (yaml, code, want) in [
            (
                "  - { capability: snapshots, id: marks, entry: snaps/gone.star }\n".to_string(),
                "implementation.script_missing",
                "isn't in the extension".to_string(),
            ),
            (
                "  - { capability: snapshots, id: marks, entry: snaps/broken.star }\n".to_string(),
                "implementation.script_invalid",
                "doesn't parse".into(),
            ),
            (
                "  - { capability: snapshots, id: marks, entry: snaps/one.star }\n".to_string(),
                "implementation.script_missing_verb",
                "defines no `changed`".into(),
            ),
            (
                snap(", features: { contents: true }"),
                "implementation.script_missing_verb",
                "defines no `read_at` (`def read_at(x):`), a verb a `snapshots` implementation answers when it declares `contents`".into(),
            ),
            (
                snap(", features: { wings: true }"),
                "implementation.script_declares",
                "`wings` isn't one of its features".into(),
            ),
            (
                "  - { capability: snapshots, id: marks, entry: snaps/s.star, needs: [threads.write] }\n".to_string(),
                "implementation.script_need",
                "threads.write".into(),
            ),
            (
                "  - { capability: snapshots, id: marks, entry: snaps/s.star, needs: [state.write:x] }\n".to_string(),
                "implementation.script_need",
                "state.write:x".into(),
            ),
            (
                "  - { capability: snapshots, id: marks, entry: snaps/s.star }\n".to_string(),
                "implementation.script_invalid",
                "without".into(),
            ),
            (
                snap(", config: [a]"),
                "implementation.script_config",
                "must be a map".into(),
            ),
            (
                snap("").replace("id: marks", "id: agent"),
                "implementation.script_id",
                "reserved".into(),
            ),
            (
                snap("").replace("snaps/s.star", "../s.star"),
                "implementation.script_outside",
                "inside the extension".into(),
            ),
            (
                "  - { capability: snapshots, id: keep, entry: \"oxplow:snapshots\", needs: [sql.read] }\n".into(),
                "implementation.built_in_needs",
                "takes no `needs`".into(),
            ),
            (
                "  - { capability: ai_provider, id: acme, provider: anthropic, needs: [sql.read] }\n".into(),
                "implementation.provider_needs",
                "takes no `needs`".into(),
            ),
        ] {
            let got = errors(&yaml);
            assert!(got.contains(code) && got.contains(&want), "{yaml}\nwant {code}: {want}\ngot {got}");
        }
    }

    /// A capability whose interface isn't a set of verbs (no provider
    /// contract) has nothing a script could define.
    #[test]
    fn a_capability_without_verbs_takes_no_script() {
        let spec = oxplow_domain::capability::spec("vcs").unwrap();
        let err = script_contract(spec, "x.star", "`vcs` implementation `jj`").unwrap_err();
        assert_eq!(err.code, "implementation.script_no_verbs");
        assert!(err.message.contains("`vcs` has none"), "{}", err.message);
    }

    /// A snapshots implementation marks a stream's worktree and keeps what
    /// it marked: it reads the tree (`tree.read`) and keeps its marks in its
    /// extension's private state (`state.write`), and a script that does so
    /// loads. Nothing about the capability is refused any more.
    #[test]
    fn a_snapshots_script_loads_with_tree_and_state_needs() {
        let script =
            "def mark(x):\n    files(\"**\")\n    scope(\"state.write\", {})\n    return {}\n\
                      def changed(x):\n    return {}\n";
        let read = |rel: &str| (rel == "s.star").then(|| script.to_string());
        let (decls, errors) = parse_with(
            "  - { capability: snapshots, id: marks, entry: s.star, needs: [tree.read, state.write, state.read] }\n",
            &read,
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            decls[0].needs,
            ["tree.read", "state.write", "state.read"].map(String::from)
        );
    }

    /// An implementation's script that calls what it didn't declare is
    /// refused at load, `tree.read` and `state.write` among them: the
    /// builtin names its need, the `scope` call needs a callable scope.
    #[test]
    fn an_undeclared_tree_read_or_state_write_is_refused_at_load() {
        let read =
            |script: &'static str| move |rel: &str| (rel == "s.star").then(|| script.to_string());
        let entry = "  - { capability: snapshots, id: marks, entry: s.star, needs: [sql.read] }\n";
        let (_, errors) = parse_with(
            entry,
            &read(
                "def mark(x):\n    files(\"**\")\n    return {}\ndef changed(x):\n    return {}\n",
            ),
        );
        let got = errors.iter().map(|d| d.message.clone()).collect::<String>();
        assert!(got.contains("without `tree.read`"), "{got}");
        // A tree.read with no body to call it: the declaration itself is fine.
        let (_, errors) = parse_with(
            "  - { capability: snapshots, id: marks, entry: s.star, needs: [tree.read] }\n",
            &read("def mark(x):\n    return {}\ndef changed(x):\n    return {}\n"),
        );
        assert!(errors.is_empty(), "{errors:?}");
    }

    const MARK: &str = "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 8 8\"><path d=\"M0 0h8v8z\"/></svg>";

    /// `icon:` is a path to an SVG inside the extension, whatever the
    /// capability; the declaration holds it as a data URI, for an `<img>`.
    #[test]
    fn an_implementation_may_declare_an_icon() {
        let read = |rel: &str| (rel == "icons/a.svg").then(|| MARK.to_string());
        let (decls, errors) = parse_with(
            "  - { capability: agent_harness, id: claude, entry: oxplow:claude-code, icon: icons/a.svg }\n",
            &read,
        );
        assert!(errors.is_empty(), "{errors:?}");
        let uri = decls[0].icon.as_deref().unwrap();
        let b64 = uri.strip_prefix("data:image/svg+xml;base64,").unwrap();
        use base64::Engine as _;
        assert_eq!(
            String::from_utf8(
                base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .unwrap()
            )
            .unwrap(),
            MARK
        );
        // Without one, none.
        let (decls, _) =
            parse("  - { capability: agent_harness, id: claude, entry: oxplow:claude-code }\n");
        assert_eq!(decls[0].icon, None);
    }

    /// The checker refuses an icon that isn't there, isn't an SVG, carries a
    /// script or is too big, each at the entry's line.
    #[test]
    fn a_bad_icon_is_refused() {
        let big = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\">{}</svg>",
            " ".repeat(20_000)
        );
        let files: std::collections::HashMap<&str, String> = [
            ("icons/ok.svg", MARK.to_string()),
            ("icons/png.svg", "not an image".to_string()),
            (
                "icons/script.svg",
                "<svg><script>alert(1)</script></svg>".to_string(),
            ),
            ("icons/big.svg", big),
            ("icons/a.png", MARK.to_string()),
        ]
        .into();
        let read = |rel: &str| files.get(rel).cloned();
        for (icon, want) in [
            ("icons/missing.svg", "isn't in the extension"),
            ("icons/png.svg", "isn't an SVG"),
            ("icons/script.svg", "script"),
            ("icons/big.svg", "too big"),
            ("icons/a.png", "must be an `.svg`"),
            ("../escape.svg", "inside the extension"),
        ] {
            let (decls, errors) = parse_with(
                &format!("  - {{ capability: agent_harness, id: claude, entry: oxplow:claude-code, icon: {icon} }}\n"),
                &read,
            );
            assert!(decls.is_empty(), "{icon}");
            assert!(
                errors
                    .iter()
                    .map(|d| d.message.clone())
                    .collect::<Vec<_>>()
                    .join("\n")
                    .contains(want),
                "{icon}: {errors:?}"
            );
        }
    }

    #[test]
    fn a_built_in_is_declared_for_its_capability() {
        let (decls, errors) =
            parse("  - { capability: snapshots, id: keep, entry: oxplow:snapshots }\n");
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            decls,
            vec![ImplementationDecl {
                capability: "snapshots".into(),
                id: "keep".into(),
                title: None,
                entry: Some("oxplow:snapshots".into()),
                provider: None,
                skills: vec![],
                config: serde_json::json!({}),
                icon: None,
                description: None,
                needs: vec![],
                script: None,
                when: None,
            }]
        );
    }

    /// A declaration's one-sentence `description:` is kept.
    #[test]
    fn an_implementation_keeps_its_description() {
        let (decls, errors) = parse(
            "  - { capability: snapshots, id: keep, entry: oxplow:snapshots, description: \"Keeps every version.\" }\n",
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            decls[0].description.as_deref(),
            Some("Keeps every version.")
        );
    }

    #[test]
    fn what_core_doesnt_know_is_refused() {
        let errors = |yaml: &str| {
            parse(yaml)
                .1
                .iter()
                .map(|d| d.message.clone())
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(
            errors("  - { capability: vcs, id: jj, entry: oxplow:snapshots }\n")
                .contains("an extension implements")
        );
        assert!(
            errors("  - { capability: snapshots, id: beads, entry: oxplow:beads }\n")
                .contains("built-ins")
        );
        assert!(
            errors("  - { capability: snapshots, id: x, entry: oxplow:claude-code }\n")
                .contains("implements `agent_harness`")
        );
        assert!(
            errors("  - { capability: snapshots, id: none, entry: oxplow:snapshots }\n")
                .contains("not `none`")
        );
        assert!(
            errors("  - { capability: snapshots, id: oxplow, entry: oxplow:snapshots }\n")
                .contains("core's own")
        );
        assert!(errors(
            "  - { capability: snapshots, id: keep, entry: oxplow:snapshots, features: [a] }\n"
        )
        .contains("features"));
        assert!(errors(
            "  - { capability: snapshots, id: keep, entry: oxplow:snapshots }\n  - { capability: snapshots, id: keep, entry: oxplow:snapshots }\n"
        )
        .contains("twice"));
    }
}
