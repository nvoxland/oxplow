//! `action_points:` — named places where several extensions offer the same
//! kind of action ("New Task") and none of them owns the page
//! (`.context/extensions-slots.md` "Action points and the store filter").
//!
//! ```yaml
//! action_points:
//!   - name: task.new
//!     label: New Task
//!     doc: Where a task is created.
//!     context: { parent?: uri, thread_id?: int }
//! ```
//!
//! The point's id is `<namespace>/<name>`. A command joins it with
//! `ui: { at: <id> }` and gets the context as its input; a page places it
//! with an `action_point` node.

use std::collections::BTreeMap;

use oxplow_domain::diagnostic::Diagnostic;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use super::manifest_v2::{at, item_lines, key_line};

const MAX_POINTS: usize = 64;

/// The type of one context key.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type, schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ContextType {
    String,
    /// A ref (`oxplow:…`): a string.
    Uri,
    Int,
    Bool,
}

impl ContextType {
    /// Whether `v` is a value of this type.
    pub fn holds(self, v: &serde_json::Value) -> bool {
        match self {
            Self::String | Self::Uri => v.is_string(),
            Self::Int => v.is_i64() || v.is_u64(),
            Self::Bool => v.is_boolean(),
        }
    }
}

/// One key of a point's context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ContextKey {
    pub name: String,
    pub ty: ContextType,
    /// Declared `name?`: a page may leave it out.
    pub optional: bool,
}

/// An action point an extension declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ActionPointDecl {
    /// `<namespace>/<name>`.
    pub id: String,
    pub label: String,
    pub doc: Option<String>,
    pub context: Vec<ContextKey>,
    /// The context key (a `uri`) naming the item it acts on: a joined
    /// command's `ui.accepts` is checked against that item's store.
    pub item: Option<String>,
}

/// The on-disk shape.
#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActionPointFile {
    name: String,
    label: String,
    #[serde(default)]
    doc: Option<String>,
    /// `{ <key>: <type> }`; a key written `name?` is optional.
    #[serde(default)]
    context: BTreeMap<String, ContextType>,
    /// The context key naming the item it acts on (a required `uri`).
    #[serde(default)]
    item: Option<String>,
}

/// `action_points:` of the extension in `namespace`: the valid points and
/// what is wrong with the rest.
pub fn parse_action_points(
    value: &Value,
    file: &str,
    manifest: &str,
    namespace: &str,
) -> (Vec<ActionPointDecl>, Vec<Diagnostic>) {
    let block = key_line(manifest, "action_points");
    let Some(items) = value.as_sequence() else {
        return (
            Vec::new(),
            vec![at(
                "action_point.not_a_list",
                file,
                block,
                "`action_points` must be a list",
            )],
        );
    };
    if items.len() > MAX_POINTS {
        return (
            Vec::new(),
            vec![at(
                "action_point.too_many",
                file,
                block,
                "`action_points` may declare at most 64 points",
            )],
        );
    }
    let lines = item_lines(manifest, "action_points");
    let mut points: Vec<ActionPointDecl> = Vec::new();
    let mut errors = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let line = lines.get(index).copied().or(block);
        let raw = match serde_yaml::from_value::<ActionPointFile>(item.clone()) {
            Ok(raw) => raw,
            Err(e) => {
                errors.push(
                    at(
                        "action_point.invalid_fields",
                        file,
                        line,
                        format!("action point declaration has invalid fields: {e}"),
                    )
                    .fix("`{ name, label, doc?, context?: { <key>: string | uri | int | bool } }`"),
                );
                continue;
            }
        };
        let id = format!("{namespace}/{}", raw.name);
        if !oxplow_domain::commands::is_action_point_id(&id) {
            errors.push(
                at(
                    "action_point.invalid_name",
                    file,
                    line,
                    format!("`{}` isn't an action point name", raw.name),
                )
                .fix("dotted lowercase words: `task.new`"),
            );
            continue;
        }
        if raw.label.trim().is_empty() {
            errors.push(at(
                "action_point.no_label",
                file,
                line,
                format!("action point `{id}` needs a `label`"),
            ));
            continue;
        }
        if points.iter().any(|p| p.id == id) {
            errors.push(at(
                "action_point.duplicate",
                file,
                line,
                format!("action point `{id}` is declared twice"),
            ));
            continue;
        }
        let mut context = Vec::new();
        let mut bad = None;
        for (key, ty) in &raw.context {
            let (name, optional) = match key.strip_suffix('?') {
                Some(name) => (name, true),
                None => (key.as_str(), false),
            };
            let word = name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                && name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
            if !word || context.iter().any(|k: &ContextKey| k.name == name) {
                bad = Some(key.clone());
                break;
            }
            context.push(ContextKey {
                name: name.to_string(),
                ty: *ty,
                optional,
            });
        }
        if let Some(key) = bad {
            errors.push(
                at(
                    "action_point.invalid_context",
                    file,
                    line,
                    format!("action point `{id}`: `{key}` isn't a context key (snake_case, once)"),
                )
                .fix("`context: { parent?: uri, thread_id?: int }`"),
            );
            continue;
        }
        if let Some(item) = &raw.item {
            let names_a_uri = context
                .iter()
                .any(|k| &k.name == item && !k.optional && matches!(k.ty, ContextType::Uri));
            if !names_a_uri {
                errors.push(
                    at(
                        "action_point.invalid_item",
                        file,
                        line,
                        format!(
                            "action point `{id}`: `item: {item}` must name a required `uri` context key"
                        ),
                    )
                    .fix("`item: uri` with `context: { uri: uri, … }`"),
                );
                continue;
            }
        }
        points.push(ActionPointDecl {
            id,
            label: raw.label,
            doc: raw.doc,
            context,
            item: raw.item,
        });
    }
    (points, errors)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> (Vec<ActionPointDecl>, Vec<Diagnostic>) {
        let value: Value = serde_yaml::from_str(yaml).unwrap();
        parse_action_points(&value, "extension.yaml", "", "acme")
    }

    /// A point's `item` names a required `uri` context key; anything else
    /// is refused.
    #[test]
    fn a_points_item_is_a_required_uri_key() {
        let (ok, errors) =
            parse("- { name: task.move, label: Move, item: uri, context: { uri: uri } }");
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(ok[0].item.as_deref(), Some("uri"));
        for bad in [
            "- { name: task.move, label: Move, item: uri, context: { uri?: uri } }",
            "- { name: task.move, label: Move, item: n, context: { n: int } }",
            "- { name: task.move, label: Move, item: gone, context: { uri: uri } }",
        ] {
            let (_, errors) = parse(bad);
            assert_eq!(errors[0].code, "action_point.invalid_item", "{bad}");
        }
    }

    #[test]
    fn a_point_has_an_id_a_label_and_a_typed_context() {
        let (points, errors) = parse(
            "- { name: task.new, label: New Task, doc: Make one, context: { parent?: uri, thread_id: int } }",
        );
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(points[0].id, "acme/task.new");
        assert_eq!(points[0].doc.as_deref(), Some("Make one"));
        let keys: Vec<_> = points[0]
            .context
            .iter()
            .map(|k| (k.name.as_str(), k.ty, k.optional))
            .collect();
        assert_eq!(
            keys,
            [
                ("parent", ContextType::Uri, true),
                ("thread_id", ContextType::Int, false)
            ]
        );
    }

    #[test]
    fn a_bad_point_is_reported_and_the_rest_kept() {
        let (points, errors) = parse(
            "- { name: Bad Name, label: X }\n- { name: a, label: ' ' }\n- { name: b, label: B, context: { x: blob } }\n- { name: ok, label: Ok }\n- { name: ok, label: Again }\n- { name: c, label: C, context: { 'Bad': string } }",
        );
        let codes: Vec<_> = errors.iter().map(|e| e.code.as_str()).collect();
        assert_eq!(
            codes,
            [
                "action_point.invalid_name",
                "action_point.no_label",
                "action_point.invalid_fields",
                "action_point.duplicate",
                "action_point.invalid_context"
            ]
        );
        assert_eq!(points.len(), 1);
    }
}
