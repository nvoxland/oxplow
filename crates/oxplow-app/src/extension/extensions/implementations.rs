//! `implementations:` in `extension.yaml` (stable): the capability
//! implementations an extension declares
//! (`.context/extensions-commands.md` "Implementations").
//!
//! ```yaml
//! implementations:
//!   - capability: work_items
//!     id: oxplow
//!     entry: oxplow:tasks
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
    /// The built-in it is (`oxplow:tasks`), or the script an effort
    /// policy is; its features are core's to say. `None` for an AI
    /// provider, which names its kind instead.
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
    /// The scopes a policy script calls (`sql.read`); empty for anything
    /// else.
    pub needs: Vec<String>,
    /// Its logo, as a `data:image/svg+xml;base64,…` URI for an `<img>`
    /// (never markup): the SVG `icon:` names inside the extension, checked
    /// by [`load_icon`].
    pub icon: Option<String>,
    /// One sentence saying what this implementation is
    /// (`v_capability_provider.description`).
    pub description: Option<String>,
    /// An effort policy written as a script: its text, when `entry` names a
    /// file of the extension rather than a built-in.
    #[serde(skip)]
    #[specta(skip)]
    pub script: Option<String>,
}

/// An `implementations:` entry as the manifest holds it.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ImplementationFile {
    capability: String,
    id: String,
    #[serde(default)]
    title: Option<String>,
    /// The built-in it is (`oxplow:tasks`), or an effort policy's script.
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
    /// The scopes a policy script calls (`sql.read`): only what a script
    /// can (`scope_calls::CALLABLE`).
    #[serde(default)]
    needs: Vec<String>,
    /// A path to an SVG in the extension (`icons/claude.svg`): its logo,
    /// shown wherever the implementation is (a session's tab). Any
    /// capability may declare one.
    #[serde(default)]
    icon: Option<String>,
    /// One sentence saying what it is.
    #[serde(default)]
    description: Option<String>,
}

/// The largest icon file.
const ICON_MAX_BYTES: usize = 16 * 1024;

/// The SVG at `path` in the extension as a data URI. It is only ever
/// drawn as an image, so a script in it would not run; one is refused all
/// the same, as is anything but a small `.svg` inside the extension.
fn load_icon(path: &str, read: &dyn Fn(&str) -> Option<String>) -> Result<String, String> {
    if path.starts_with('/') || path.split('/').any(|p| p == "..") {
        return Err(format!("icon `{path}` must be inside the extension"));
    }
    if !path.to_ascii_lowercase().ends_with(".svg") {
        return Err(format!("icon `{path}` must be an `.svg` file"));
    }
    let Some(text) = read(path) else {
        return Err(format!("icon `{path}` isn't in the extension"));
    };
    if text.len() > ICON_MAX_BYTES {
        return Err(format!(
            "icon `{path}` is too big ({} bytes; at most {ICON_MAX_BYTES})",
            text.len()
        ));
    }
    let body = text.trim_start();
    let body = body
        .strip_prefix("<?xml")
        .and_then(|rest| rest.split_once("?>"))
        .map_or(body, |(_, rest)| rest.trim_start());
    if !body.starts_with("<svg") {
        return Err(format!(
            "icon `{path}` isn't an SVG (it must start with `<svg`)"
        ));
    }
    if text.to_ascii_lowercase().contains("<script") {
        return Err(format!(
            "icon `{path}` carries a script; an icon is only a picture"
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
) -> (Vec<ImplementationDecl>, Vec<String>) {
    let block = key_line(manifest, "implementations");
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(file, block, "`implementations` must be a list")],
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
                errors.push(at(file, item_line, format!("implementation: {e}")));
                continue;
            }
        };
        let line = item_line;
        match decl_of(f, read) {
            Ok(d)
                if out
                    .iter()
                    .any(|o| o.capability == d.capability && o.id == d.id) =>
            {
                errors.push(at(
                    file,
                    line,
                    format!(
                        "`{}` implementation `{}` is declared twice",
                        d.capability, d.id
                    ),
                ))
            }
            Ok(d) => out.push(d),
            Err(e) => errors.push(at(file, line, e)),
        }
    }
    (out, errors)
}

fn decl_of(
    f: ImplementationFile,
    read: &dyn Fn(&str) -> Option<String>,
) -> Result<ImplementationDecl, String> {
    use oxplow_domain::capability;
    let Some(spec) = capability::spec(&f.capability).filter(|c| c.choosable || c.many) else {
        return Err(format!(
            "`{}` isn't a capability an extension implements ({})",
            f.capability,
            capability::CAPABILITIES
                .iter()
                .filter(|c| c.choosable || c.many)
                .map(|c| c.id)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    };
    // A required capability's default is core's own; a many-capability's
    // default is just the one used when nothing names one.
    if !spec.optional && !spec.many && f.id == spec.default {
        return Err(format!(
            "`{}` implementation `{}` is core's own (a required capability's default); \
             declare another id",
            spec.id, f.id
        ));
    }
    if f.id == capability::NONE
        || f.id.is_empty()
        || !f
            .id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(format!(
            "implementation id `{}` must be lowercase letters, digits and `_`, and not `none`",
            f.id
        ));
    }
    let icon = f.icon.as_deref().map(|p| load_icon(p, read)).transpose()?;
    if f.capability == "ai_provider" {
        return provider_decl(f, icon);
    }
    if f.provider.is_some() {
        return Err(format!(
            "`provider:` names an AI provider's kind; a `{}` implementation names an `entry:`",
            f.capability
        ));
    }
    let Some(entry) = f.entry.clone() else {
        return Err(format!(
            "`{}` implementation `{}` needs an `entry:`",
            f.capability, f.id
        ));
    };
    if !entry.starts_with("oxplow:") {
        return script_decl(f, &entry, read, icon);
    }
    if !f.needs.is_empty() {
        return Err(format!(
            "entry `{entry}` takes no `needs` (a built-in is core's code)"
        ));
    }
    let Some(built_in) = crate::capabilities::built_in(&entry) else {
        return Err(format!(
            "entry `{entry}` isn't one of core's built-ins ({})",
            crate::capabilities::BUILT_INS
                .iter()
                .map(|b| b.entry)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    };
    if built_in.capability != spec.id {
        return Err(format!(
            "entry `{entry}` implements `{}`, not `{}`",
            built_in.capability, spec.id
        ));
    }
    if let Some(provider) = built_in.provider.filter(|p| *p != f.id) {
        return Err(format!(
            "entry `{entry}` is declared under `{provider}`, the provider its items' refs carry, not `{}`",
            f.id
        ));
    }
    let config = match f.config {
        Some(v) => serde_json::to_value(v).map_err(|e| format!("config: {e}"))?,
        None => serde_json::json!({}),
    };
    match built_in.config_schema {
        Some(schema) => capability::check_config(schema, &config)
            .map_err(|e| format!("`{}` implementation `{}`: {e}", spec.id, f.id))?,
        None if config != serde_json::json!({}) => {
            return Err(format!("entry `{entry}` takes no config"))
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
        needs: Vec::new(),
        icon,
        description: f.description,
        script: None,
    })
}

/// An AI provider: the Rust kind it is (`provider:`, one of
/// `oxplow_ai_rig::KINDS`) and its `config: { baseUrl? }`, the API base
/// an instance that names none uses.
fn provider_decl(
    f: ImplementationFile,
    icon: Option<String>,
) -> Result<ImplementationDecl, String> {
    let known = || oxplow_ai_rig::names().join(", ");
    if f.entry.is_some() {
        return Err(format!(
            "`ai_provider` implementation `{}` names its kind with `provider:` ({}), not `entry:`",
            f.id,
            known()
        ));
    }
    if !f.needs.is_empty() {
        return Err(format!(
            "`ai_provider` implementation `{}` takes no `needs` (it calls nothing of oxplow's)",
            f.id
        ));
    }
    let Some(kind) = f.provider else {
        return Err(format!(
            "`ai_provider` implementation `{}` needs `provider:`, the kind it is ({})",
            f.id,
            known()
        ));
    };
    oxplow_ai_rig::kind(&kind)
        .map_err(|e| format!("`ai_provider` implementation `{}`: {e}", f.id))?;
    let config = match f.config {
        Some(v) => serde_json::to_value(v).map_err(|e| format!("config: {e}"))?,
        None => serde_json::json!({}),
    };
    let bad = |what: String| format!("`ai_provider` implementation `{}`: {what}", f.id);
    let Some(fields) = config.as_object() else {
        return Err(bad("`config` must be a map".into()));
    };
    for (key, value) in fields {
        match key.as_str() {
            "baseUrl" if value.is_string() => {}
            "baseUrl" => return Err(bad("`config.baseUrl` must be a URL".into())),
            other => {
                return Err(bad(format!(
                    "config: unknown key `{other}` (an AI provider takes `baseUrl`)"
                )))
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
        needs: Vec::new(),
        icon,
        description: f.description,
        script: None,
    })
}

/// An effort policy whose entry is a script in the extension, checked
/// as it will be registered: an effects-shaped `transform`, the scopes it
/// calls in `needs`.
fn script_decl(
    f: ImplementationFile,
    entry: &str,
    read: &dyn Fn(&str) -> Option<String>,
    icon: Option<String>,
) -> Result<ImplementationDecl, String> {
    let config = match &f.config {
        Some(v) => serde_json::to_value(v).map_err(|e| format!("config: {e}"))?,
        None => serde_json::json!({}),
    };
    if f.capability != "effort_policy" {
        return Err(format!(
            "entry `{entry}` is a script, and a script implements an `effort_policy` (others \
             name a built-in: `oxplow:<name>`; an `ai_provider` names its kind with `provider:`)"
        ));
    }
    if config != serde_json::json!({}) {
        return Err(format!(
            "`effort_policy` implementation `{}`: a policy script takes no config",
            f.id
        ));
    }
    for need in &f.needs {
        crate::scope_calls::check_script_need(need).map_err(|e| {
            format!(
                "`effort_policy` implementation `{}`: needs `{need}`: {e}",
                f.id
            )
        })?;
    }
    let script = read(entry).ok_or_else(|| format!("entry `{entry}` isn't in the extension"))?;
    oxplow_script::runtime::check_starlark(entry, &script)
        .and_then(|()| {
            crate::services::check_uses(&script, &f.needs, crate::scope_calls::check_script_need)
        })
        .map_err(|e| format!("`effort_policy` implementation `{}`: {e}", f.id))?;
    Ok(ImplementationDecl {
        capability: f.capability,
        id: f.id,
        title: f.title,
        entry: Some(entry.to_string()),
        provider: None,
        skills: f.skills,
        config,
        needs: f.needs,
        icon,
        description: f.description,
        script: Some(script),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> (Vec<ImplementationDecl>, Vec<String>) {
        parse_with(yaml, &|_| None)
    }

    fn parse_with(
        yaml: &str,
        read: &dyn Fn(&str) -> Option<String>,
    ) -> (Vec<ImplementationDecl>, Vec<String>) {
        let manifest = format!("implementations:\n{yaml}");
        let doc: Value = serde_yaml::from_str(&manifest).unwrap();
        parse_implementations(&doc["implementations"], "extension.yaml", &manifest, read)
    }

    /// An AI provider names the Rust kind it is with `provider:` (no
    /// `entry:`, no script) and takes `config: { baseUrl? }`. An unknown
    /// kind is refused, naming the known ones.
    #[test]
    fn an_ai_provider_names_its_kind() {
        let (decls, errors) = parse(
            "  - { capability: ai_provider, id: acme, title: Acme, provider: openai_compatible, config: { baseUrl: \"https://api.acme.test\" } }\n",
        );
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(decls[0].provider.as_deref(), Some("openai_compatible"));
        assert_eq!(decls[0].entry, None);
        assert_eq!(decls[0].script, None);
        assert_eq!(decls[0].config["baseUrl"], "https://api.acme.test");
        let errors = |yaml: &str| parse(yaml).1.join("\n");
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
                "  - { capability: ai_provider, id: acme, provider: anthropic, needs: [sql.read] }\n",
                "takes no `needs`",
            ),
            (
                "  - { capability: work_items, id: acme, provider: anthropic, entry: oxplow:tasks }\n",
                "`provider:` names an AI provider's kind",
            ),
        ] {
            let got = errors(yaml);
            assert!(got.contains(want), "{yaml}: {got}");
        }
    }

    const POLICY: &str = "def transform(x):\n    return {\"skip\": \"nothing\"}\n";

    /// An effort policy may be a script too: an effects-shaped
    /// `transform`, with the scopes it calls in its `needs`. It takes no
    /// config; a need a script can't call, a script without `transform`
    /// and a missing file are refused, and so is `needs` on anything but a
    /// policy script.
    #[test]
    fn an_effort_policy_may_be_a_script_with_its_needs() {
        let read = |rel: &str| match rel {
            "policies/p.star" => Some(POLICY.to_string()),
            "policies/bare.star" => Some("def other(x):\n    return None\n".to_string()),
            _ => None,
        };
        let (decls, errors) = parse_with(
            "  - { capability: effort_policy, id: acme, title: Acme, entry: policies/p.star, needs: [sql.read] }\n",
            &read,
        );
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(decls[0].script.as_deref(), Some(POLICY));
        assert_eq!(decls[0].needs, vec!["sql.read".to_string()]);
        let errors = |yaml: &str| parse_with(yaml, &read).1.join("\n");
        for (yaml, want) in [
            (
                "  - { capability: effort_policy, id: acme, entry: policies/p.star, needs: [threads.write] }\n",
                "threads.write",
            ),
            (
                "  - { capability: effort_policy, id: acme, entry: policies/bare.star }\n",
                "transform",
            ),
            (
                "  - { capability: effort_policy, id: acme, entry: policies/gone.star }\n",
                "isn't in the extension",
            ),
            (
                "  - { capability: effort_policy, id: acme, entry: policies/p.star, config: { a: 1 } }\n",
                "takes no config",
            ),
            (
                "  - { capability: effort_policy, id: tidy, entry: \"oxplow:commit-or-switch\", needs: [sql.read] }\n",
                "takes no `needs`",
            ),
            (
                "  - { capability: work_items, id: acme, entry: policies/p.star }\n",
                "a script implements an `effort_policy`",
            ),
        ] {
            let got = errors(yaml);
            assert!(got.contains(want), "{yaml}: {got}");
        }
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
        assert_eq!(errors, Vec::<String>::new());
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
            assert!(errors.join("\n").contains(want), "{icon}: {errors:?}");
        }
    }

    #[test]
    fn a_built_in_is_declared_for_its_capability() {
        let (decls, errors) =
            parse("  - { capability: work_items, id: oxplow, entry: oxplow:tasks }\n");
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(
            decls,
            vec![ImplementationDecl {
                capability: "work_items".into(),
                id: "oxplow".into(),
                title: None,
                entry: Some("oxplow:tasks".into()),
                provider: None,
                skills: vec![],
                config: serde_json::json!({}),
                needs: vec![],
                script: None,
                icon: None,
                description: None,
            }]
        );
    }

    /// A declaration's one-sentence `description:` is kept.
    #[test]
    fn an_implementation_keeps_its_description() {
        let (decls, errors) = parse(
            "  - { capability: work_items, id: oxplow, entry: oxplow:tasks, description: \"The work list oxplow keeps.\" }\n",
        );
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(
            decls[0].description.as_deref(),
            Some("The work list oxplow keeps.")
        );
    }

    /// A built-in whose items' refs carry a provider (`oxplow:oxplow/work_item?id=…&provider=oxplow`)
    /// is declared under that id and no other: its refs would otherwise
    /// name a list that refuses them.
    #[test]
    fn a_built_in_is_declared_under_the_provider_its_refs_carry() {
        let (decls, errors) =
            parse("  - { capability: work_items, id: tasks, entry: oxplow:tasks }\n");
        assert!(decls.is_empty());
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert!(
            errors[0].contains("`oxplow:tasks`") && errors[0].contains("`oxplow`"),
            "{}",
            errors[0]
        );
    }

    /// A declaration's `config:` is checked against its built-in's schema;
    /// one that takes none (every built-in today) refuses any.
    #[test]
    fn config_is_checked_against_the_built_ins_schema() {
        let errors = |yaml: &str| parse(yaml).1.join("\n");
        assert!(errors(
            "  - { capability: agent_harness, id: claude, entry: oxplow:claude-code, config: { a: 1 } }\n"
        )
        .contains("takes no config"));
        // A many-capability's default id is anyone's to declare.
        assert_eq!(
            parse("  - { capability: agent_harness, id: claude, entry: oxplow:claude-code }\n").1,
            Vec::<String>::new()
        );
    }

    #[test]
    fn what_core_doesnt_know_is_refused() {
        let errors = |yaml: &str| parse(yaml).1.join("\n");
        assert!(
            errors("  - { capability: vcs, id: jj, entry: oxplow:tasks }\n")
                .contains("an extension implements")
        );
        assert!(
            errors("  - { capability: work_items, id: beads, entry: oxplow:beads }\n")
                .contains("built-ins")
        );
        assert!(
            errors("  - { capability: snapshots, id: x, entry: oxplow:tasks }\n")
                .contains("implements `work_items`")
        );
        assert!(
            errors("  - { capability: work_items, id: none, entry: oxplow:tasks }\n")
                .contains("not `none`")
        );
        assert!(
            errors("  - { capability: snapshots, id: oxplow, entry: oxplow:snapshots }\n")
                .contains("core's own")
        );
        assert!(errors(
            "  - { capability: work_items, id: oxplow, entry: oxplow:tasks, features: [a] }\n"
        )
        .contains("features"));
        assert!(errors(
            "  - { capability: work_items, id: oxplow, entry: oxplow:tasks }\n  - { capability: work_items, id: oxplow, entry: oxplow:tasks }\n"
        )
        .contains("twice"));
    }
}
