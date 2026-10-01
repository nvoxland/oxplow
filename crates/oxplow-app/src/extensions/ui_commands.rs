//! `ui.commands` (P6b.C4): commands an extension adds to core menus,
//! scoped to a ref — the page nav bar's menu for the page's ref (`menu`)
//! and a row's right-click menu for the row's ref (`context`).
//!
//! ```yaml
//! ui:
//!   commands:
//!     - { command: fake.comment, label: "Comment in Fake…", about: work_item, placement: [menu, context] }
//!     - { command: work_item.transition, label: Move to Done, about: work_item, input: { ref: "{{ref}}", to: done } }
//! ```
//!
//! A launcher has no current ref, so there's no launcher placement:
//! `launcher: [{ target: { command } }]` covers that.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::manifest_v2::{at, key_line, line_under};
use super::placeholders;

/// Where a command shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum UiPlacement {
    /// The page nav bar's menu, for the page's ref.
    Menu,
    /// A row's right-click menu, for the row's ref.
    Context,
}

/// A command in core menus (valid ones; invalid ones are in the
/// extension's `errors`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct UiCommand {
    /// `<extension>/<n>`, its place in the list.
    pub id: String,
    pub extension: String,
    /// What the menu groups it under: the provider's id when the command
    /// is one of the extension's providers', else the extension's name.
    pub group: String,
    pub command: String,
    pub label: String,
    /// The ref kind it acts on (`work_item`, `commit`).
    pub about: String,
    pub placement: Vec<UiPlacement>,
    /// The command's input; whole-value `{{ref}}` / `{{ref.id}}` strings
    /// are the ref it runs for and its id.
    #[specta(type = oxplow_domain::Json)]
    pub input: Value,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UiCommandFile {
    command: String,
    label: String,
    about: String,
    #[serde(default)]
    placement: Option<Vec<UiPlacement>>,
    #[serde(default)]
    input: Option<Value>,
}

/// The placeholders an input may use: the ref, and its id.
fn placeholder_problem(input: &Value) -> Option<String> {
    fn walk(v: &Value) -> Option<String> {
        match v {
            Value::String(s) => {
                let found = placeholders(s);
                if found.is_empty() {
                    return None;
                }
                let whole = super::whole_placeholder(s);
                match whole {
                    Some(p) if p.scope == "ref" && (p.name.is_empty() || p.name == "id") => None,
                    _ => Some(format!(
                        "`{s}`: an input may use `{{{{ref}}}}` or `{{{{ref.id}}}}`, as a whole value"
                    )),
                }
            }
            Value::Array(items) => items.iter().find_map(walk),
            Value::Object(map) => map.values().find_map(walk),
            _ => None,
        }
    }
    walk(input)
}

/// Parse `ui.commands`: the valid entries, and an error (`file:line: …`)
/// for each broken one. `providers` are the extension's provider ids.
pub fn parse_ui_commands(
    extension: &str,
    providers: &[String],
    value: &serde_yaml::Value,
    file: &str,
    manifest: &str,
) -> (Vec<UiCommand>, Vec<String>) {
    let block = line_under(manifest, "ui", "commands:").or(key_line(manifest, "ui"));
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(file, block, "`ui.commands` must be a list")],
        );
    };
    let kinds = oxplow_domain::refs::kind::core_kinds();
    let mut out = Vec::new();
    let mut errors = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let entry: UiCommandFile = match serde_yaml::from_value(item.clone()) {
            Ok(e) => e,
            Err(e) => {
                errors.push(at(file, block, format!("`ui.commands`: {e}")));
                continue;
            }
        };
        let line = line_under(manifest, "ui", &format!("command: {}", entry.command)).or(block);
        let input = entry.input.unwrap_or_else(|| json!({ "ref": "{{ref}}" }));
        let problem = if let Err(e) = oxplow_domain::CommandSpec::validate_name(&entry.command) {
            Some(e.to_string())
        } else if kinds.get(&entry.about).is_none() {
            Some(format!(
                "`about: {}` isn't a kind of ref (`work_item`, `commit`, `file`, …)",
                entry.about
            ))
        } else if entry.placement.as_ref().is_some_and(|p| p.is_empty()) {
            Some("`placement` is empty: `menu`, `context`, or both".into())
        } else if !input.is_object() {
            Some("`input` must be a map".into())
        } else {
            placeholder_problem(&input)
        };
        if let Some(p) = problem {
            errors.push(at(
                file,
                line,
                format!("`ui.commands` `{}`: {p}", entry.label),
            ));
            continue;
        }
        let namespace = entry.command.split('.').next().unwrap_or_default();
        let group = if providers.iter().any(|p| p == namespace) {
            namespace.to_string()
        } else {
            extension.to_string()
        };
        out.push(UiCommand {
            id: format!("{extension}/{i}"),
            extension: extension.to_string(),
            group,
            command: entry.command,
            label: entry.label,
            about: entry.about,
            placement: entry
                .placement
                .unwrap_or_else(|| vec![UiPlacement::Menu, UiPlacement::Context]),
            input,
        });
    }
    (out, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str, providers: &[&str]) -> (Vec<UiCommand>, Vec<String>) {
        let manifest = format!("manifest: 2\nname: x\nui:\n  commands:\n{yaml}");
        let m: super::super::manifest_v2::ManifestV2 = serde_yaml::from_str(&manifest).unwrap();
        let providers: Vec<String> = providers.iter().map(|p| p.to_string()).collect();
        parse_ui_commands(
            "x",
            &providers,
            m.ui.commands.as_ref().unwrap(),
            "x/extension.yaml",
            &manifest,
        )
    }

    #[test]
    fn commands_group_by_provider_and_default_to_the_ref() {
        let (cmds, errors) = parse(
            "    - { command: fake.comment, label: Comment in Fake…, about: work_item }\n    - { command: work_item.transition, label: Done, about: work_item, placement: [context], input: { ref: \"{{ref}}\", to: done } }\n",
            &["fake"],
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(cmds[0].group, "fake", "one of the extension's providers");
        assert_eq!(cmds[0].input, json!({ "ref": "{{ref}}" }));
        assert_eq!(
            cmds[0].placement,
            vec![UiPlacement::Menu, UiPlacement::Context]
        );
        assert_eq!(cmds[1].group, "x", "anything else: the extension");
        assert_eq!(cmds[1].placement, vec![UiPlacement::Context]);
        assert_eq!(cmds[1].id, "x/1");
    }

    #[test]
    fn a_broken_entry_is_an_error_at_its_line() {
        for (entry, says) in [
            ("{ command: Bad, label: L, about: work_item }", "`Bad`"),
            (
                "{ command: a.b, label: L, about: nonsense }",
                "isn't a kind of ref",
            ),
            (
                "{ command: a.b, label: L, about: commit, placement: [] }",
                "`placement` is empty",
            ),
            (
                "{ command: a.b, label: L, about: commit, input: [1] }",
                "must be a map",
            ),
            (
                "{ command: a.b, label: L, about: commit, input: { sha: \"x{{ref.id}}\" } }",
                "as a whole value",
            ),
            (
                "{ command: a.b, label: L, about: commit, input: { sha: \"{{row.id}}\" } }",
                "as a whole value",
            ),
            (
                "{ command: a.b, label: L, about: commit, placement: [launcher] }",
                "unknown variant",
            ),
        ] {
            let (cmds, errors) = parse(&format!("    - {entry}\n"), &[]);
            let errs = errors.join("\n");
            assert!(
                errs.contains(says) && errs.contains("x/extension.yaml:"),
                "{entry}: {errs}"
            );
            assert!(cmds.is_empty(), "{entry}");
        }
    }
}
