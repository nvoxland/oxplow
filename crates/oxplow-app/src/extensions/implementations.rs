//! `implementations:` in `extension.yaml` (stable): the capability
//! implementations an extension declares
//! (`.context/extensions.md` "Implementations").
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
//! many implementations serve (`agent_harness`, `acp_adapter`,
//! `ai_provider`), every one declared. `config:` configures the built-in,
//! checked against its schema:
//!
//! ```yaml
//!   - capability: acp_adapter
//!     id: gemini
//!     entry: oxplow:acp-adapter
//!     config: { command: gemini, args: [--acp] }
//! ```

use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use super::manifest_v2::{at, entry_line, key_line};

/// One implementation as the extension declares it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ImplementationDecl {
    pub capability: String,
    /// What `activeProviders` names.
    pub id: String,
    /// How a person names it; the built-in's own title when absent.
    pub title: Option<String>,
    /// The built-in it is (`oxplow:tasks`); its features are core's to
    /// say.
    pub entry: String,
    /// The extension's skills (`skills:`) it owns: offered only while it's
    /// the active implementation.
    pub skills: Vec<String>,
    /// What the declaration configures, checked against the built-in's
    /// schema; `{}` when it says nothing.
    #[specta(type = specta_typescript::Any)]
    pub config: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ImplementationFile {
    capability: String,
    id: String,
    #[serde(default)]
    title: Option<String>,
    entry: String,
    #[serde(default)]
    skills: Vec<String>,
    #[serde(default)]
    config: Option<Value>,
}

/// Parse `implementations:`: the valid declarations, and what's wrong
/// with the rest (at their lines).
pub fn parse_implementations(
    value: &Value,
    file: &str,
    manifest: &str,
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
    for item in items {
        let f: ImplementationFile = match serde_yaml::from_value(item.clone()) {
            Ok(f) => f,
            Err(e) => {
                errors.push(at(file, block, format!("implementation: {e}")));
                continue;
            }
        };
        let line = entry_line(manifest, "implementations", "id", &f.id).or(block);
        match decl_of(f) {
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

fn decl_of(f: ImplementationFile) -> Result<ImplementationDecl, String> {
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
    let Some(built_in) = crate::capabilities::built_in(&f.entry) else {
        return Err(format!(
            "entry `{}` isn't one of core's built-ins ({})",
            f.entry,
            crate::capabilities::BUILT_INS
                .iter()
                .map(|b| b.entry)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    };
    if built_in.capability != spec.id {
        return Err(format!(
            "entry `{}` implements `{}`, not `{}`",
            f.entry, built_in.capability, spec.id
        ));
    }
    if let Some(provider) = built_in.provider.filter(|p| *p != f.id) {
        return Err(format!(
            "entry `{}` is declared under `{provider}`, the provider its items' refs carry, not `{}`",
            f.entry, f.id
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
            return Err(format!("entry `{}` takes no config", f.entry))
        }
        None => {}
    }
    Ok(ImplementationDecl {
        capability: f.capability,
        id: f.id,
        title: f.title,
        entry: f.entry,
        skills: f.skills,
        config,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> (Vec<ImplementationDecl>, Vec<String>) {
        let manifest = format!("implementations:\n{yaml}");
        let doc: Value = serde_yaml::from_str(&manifest).unwrap();
        parse_implementations(&doc["implementations"], "extension.yaml", &manifest)
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
                entry: "oxplow:tasks".into(),
                skills: vec![],
                config: serde_json::json!({}),
            }]
        );
    }

    /// A built-in whose items' refs carry a provider (`work_item:oxplow:…`)
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
    /// one that takes none refuses any.
    #[test]
    fn config_is_checked_against_the_built_ins_schema() {
        let (decls, errors) = parse(
            "  - { capability: acp_adapter, id: gemini, entry: oxplow:acp-adapter, config: { command: gemini, args: [--acp] } }\n",
        );
        assert_eq!(errors, Vec::<String>::new());
        assert_eq!(
            decls[0].config,
            serde_json::json!({ "command": "gemini", "args": ["--acp"] })
        );
        let errors = |yaml: &str| parse(yaml).1.join("\n");
        assert!(
            errors("  - { capability: acp_adapter, id: x, entry: oxplow:acp-adapter, config: { args: [] } }\n")
                .contains("command"),
        );
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
