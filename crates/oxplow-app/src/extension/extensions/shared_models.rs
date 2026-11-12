//! `models:`, `fills:` and `fields:` — shared models an extension declares,
//! the models it fills as a store, and the fields it adds to another's
//! items (`.context/models.md`).
//!
//! ```yaml
//! models:
//!   - name: book
//!     resource: book            # this extension's resource (resources:)
//!     open: true
//!     key: id
//!     change_event: shelf.book.changed
//!     columns:
//!       id: text
//!       title: { type: text, doc: The title. }
//!       body: { type: text, markdown: { pins: true } }
//!       about: { type: uri, edge: book.about }
//! fills:
//!   - model: shelf/book
//!     stores: [shelf-folder]
//!     fields: [{ name: rating, title: Rating, kind: number }]
//! fields:
//!   - model: shelf/book
//!     fields: [{ name: score, title: Score, kind: number }]
//! ```
//!
//! A model's change event is declared here, not in `event_types:`: its
//! payload is core's ([`crate::models::change_event_schema`]), so it is
//! added to the extension's declared types with that schema.

use std::collections::BTreeMap;

use oxplow_db::model_store::{FieldDecl, Markdown, ModelColumn, ModelDecl, ModelType};
use oxplow_domain::diagnostic::Diagnostic;
use serde::Deserialize;
use serde_yaml::Value;

use super::manifest_v2::{at, item_lines, key_line};
use crate::models::{FieldsSpec, FillSpec, ModelSpec};

/// What an extension's three keys declare, the valid parts.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, Deserialize)]
pub struct Declared {
    pub models: Vec<ModelSpec>,
    pub fills: Vec<FillSpec>,
    pub fields: Vec<FieldsSpec>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelFile {
    name: String,
    resource: String,
    #[serde(default)]
    open: bool,
    key: String,
    #[serde(default)]
    store_param: Option<String>,
    #[serde(default)]
    change_event: Option<String>,
    #[schemars(with = "BTreeMap<String, serde_json::Value>")]
    columns: BTreeMap<String, Value>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ColumnFile {
    #[serde(rename = "type")]
    ty: String,
    #[serde(default)]
    doc: Option<String>,
    #[serde(default)]
    values: Option<Vec<String>>,
    #[serde(default)]
    markdown: Option<MarkdownFile>,
    #[serde(default)]
    edge: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct MarkdownFile {
    #[serde(default)]
    pins: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FieldFile {
    name: String,
    title: String,
    kind: String,
    #[serde(default)]
    values: Option<Vec<String>>,
    #[serde(default)]
    read_only: bool,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FillFile {
    model: String,
    #[serde(default)]
    stores: Option<Vec<String>>,
    /// How its ids look in text (bare links; not used yet).
    #[serde(default)]
    id_pattern: Option<String>,
    /// A task store's stages (bundled's task model; not used yet).
    #[serde(default)]
    #[schemars(with = "Option<serde_json::Value>")]
    stages: Option<Value>,
    #[serde(default)]
    fields: Vec<FieldFile>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FieldsFile {
    model: String,
    fields: Vec<FieldFile>,
}

fn word(s: &str) -> bool {
    let mut c = s.chars();
    matches!(c.next(), Some(ch) if ch.is_ascii_lowercase())
        && c.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
}

fn store_id(s: &str) -> bool {
    let mut c = s.chars();
    matches!(c.next(), Some(ch) if ch.is_ascii_lowercase())
        && c.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-' || ch == '_')
}

fn column_type(ty: &str) -> Option<ModelType> {
    Some(match ty {
        "text" => ModelType::Text,
        "int" => ModelType::Int,
        "real" => ModelType::Real,
        "bool" => ModelType::Bool,
        "time" => ModelType::Time,
        "uri" => ModelType::Uri,
        "uris" => ModelType::Uris,
        _ => return None,
    })
}

fn field_decl(f: FieldFile) -> Result<FieldDecl, String> {
    if !word(&f.name) {
        return Err(format!("`{}` isn't a field name (snake_case)", f.name));
    }
    match (f.kind.as_str(), &f.values) {
        ("enum", Some(v)) if !v.is_empty() => {}
        ("enum", _) => return Err(format!("field `{}`: an enum needs its `values`", f.name)),
        ("text" | "number", _) => {}
        (other, _) => {
            return Err(format!(
                "field `{}`: kind `{other}` isn't enum, text or number",
                f.name
            ))
        }
    }
    Ok(FieldDecl {
        name: f.name,
        title: f.title,
        kind: f.kind,
        values: f.values,
        read_only: f.read_only,
    })
}

fn fields_of(list: Vec<FieldFile>) -> Result<Vec<FieldDecl>, String> {
    let mut out: Vec<FieldDecl> = Vec::new();
    for f in list {
        let d = field_decl(f)?;
        if out.iter().any(|o| o.name == d.name) {
            return Err(format!("field `{}` is declared twice", d.name));
        }
        out.push(d);
    }
    Ok(out)
}

fn model_of(raw: ModelFile, extension: &str, namespace: &str) -> Result<ModelSpec, String> {
    if !word(&raw.name) {
        return Err(format!("`{}` isn't a model name (snake_case)", raw.name));
    }
    let mut columns = Vec::new();
    for (name, v) in raw.columns {
        let col = match &v {
            Value::String(ty) => ColumnFile {
                ty: ty.clone(),
                doc: None,
                values: None,
                markdown: None,
                edge: None,
            },
            other => serde_yaml::from_value::<ColumnFile>(other.clone())
                .map_err(|e| format!("column `{name}`: {e}"))?,
        };
        let ty = column_type(&col.ty).ok_or_else(|| {
            format!(
                "column `{name}`: type `{}` isn't text, int, real, bool, time, uri or uris",
                col.ty
            )
        })?;
        if col.markdown.is_some() && ty != ModelType::Text {
            return Err(format!(
                "column `{name}`: only a text column can be markdown"
            ));
        }
        if col.values.is_some() && ty != ModelType::Text {
            return Err(format!(
                "column `{name}`: only a text column can have values"
            ));
        }
        if col.edge.is_some() && !matches!(ty, ModelType::Uri | ModelType::Uris) {
            return Err(format!(
                "column `{name}`: only a uri or uris column can be an edge"
            ));
        }
        columns.push(ModelColumn {
            name,
            ty,
            values: col.values,
            markdown: col.markdown.map(|m| Markdown { pins: m.pins }),
            edge: col.edge,
            doc: col.doc.unwrap_or_default(),
        });
    }
    if let Some(event) = &raw.change_event {
        if !event.starts_with(&format!("{namespace}.")) {
            return Err(format!(
                "change_event `{event}` must be in this extension's namespace (`{namespace}.…`)"
            ));
        }
    }
    let decl = ModelDecl {
        extension: extension.to_string(),
        namespace: namespace.to_string(),
        name: raw.name,
        resource: format!("{namespace}/{}", raw.resource),
        key: raw.key,
        store_param: raw.store_param.unwrap_or_else(|| "store".into()),
        columns,
    };
    // The store's rules for names, key and implicit columns.
    oxplow_db::model_store::check_decl(&decl).map_err(|e| e.to_string())?;
    Ok(ModelSpec {
        decl,
        open: raw.open,
        change_event: raw.change_event,
    })
}

fn list<'a>(
    value: &'a Value,
    key: &str,
    file: &str,
    manifest: &str,
    errors: &mut Vec<Diagnostic>,
) -> Option<&'a Vec<Value>> {
    let items = value.as_sequence();
    if items.is_none() {
        errors.push(at(
            &format!("{key}.not_a_list"),
            file,
            key_line(manifest, key),
            format!("`{key}` must be a list"),
        ));
    }
    items
}

/// The extension's `models:`, `fills:` and `fields:`: what is valid, and
/// what is wrong with the rest (at its line).
pub fn parse(
    models: Option<&Value>,
    fills: Option<&Value>,
    fields: Option<&Value>,
    extension: &str,
    namespace: &str,
    file: &str,
    manifest: &str,
) -> (Declared, Vec<Diagnostic>) {
    let mut out = Declared::default();
    let mut errors = Vec::new();
    if let Some(items) = models.and_then(|v| list(v, "models", file, manifest, &mut errors)) {
        let lines = item_lines(manifest, "models");
        for (i, item) in items.iter().enumerate() {
            let line = lines.get(i).copied().or(key_line(manifest, "models"));
            let parsed = serde_yaml::from_value::<ModelFile>(item.clone())
                .map_err(|e| e.to_string())
                .and_then(|raw| model_of(raw, extension, namespace));
            match parsed {
                Ok(m) if out.models.iter().any(|o| o.decl.name == m.decl.name) => errors.push(at(
                    "model.duplicate",
                    file,
                    line,
                    format!("model `{}` is declared twice", m.decl.name),
                )),
                Ok(m) => out.models.push(m),
                Err(e) => errors.push(
                    at("model.invalid", file, line, format!("model: {e}"))
                        .fix("`{ name, resource, key, columns: { <name>: <type> | { type, doc?, values?, markdown?, edge? } }, open?, change_event?, store_param? }`"),
                ),
            }
        }
    }
    if let Some(items) = fills.and_then(|v| list(v, "fills", file, manifest, &mut errors)) {
        let lines = item_lines(manifest, "fills");
        for (i, item) in items.iter().enumerate() {
            let line = lines.get(i).copied().or(key_line(manifest, "fills"));
            let parsed = serde_yaml::from_value::<FillFile>(item.clone())
                .map_err(|e| e.to_string())
                .and_then(|raw| {
                    let stores = raw.stores.unwrap_or_else(|| vec![namespace.to_string()]);
                    if let Some(bad) = stores.iter().find(|s| !store_id(s)) {
                        return Err(format!("`{bad}` isn't a store id"));
                    }
                    let _ = (raw.id_pattern, raw.stages);
                    Ok(FillSpec {
                        extension: extension.to_string(),
                        model: raw.model,
                        stores,
                        fields: fields_of(raw.fields)?,
                    })
                });
            match parsed {
                Ok(f) => out.fills.push(f),
                Err(e) => errors.push(
                    at("fill.invalid", file, line, format!("fills: {e}"))
                        .fix("`{ model: <owner ns>/<model>, stores?: [ids], fields?: [{ name, title, kind: enum|text|number, values? }] }`"),
                ),
            }
        }
    }
    if let Some(items) = fields.and_then(|v| list(v, "fields", file, manifest, &mut errors)) {
        let lines = item_lines(manifest, "fields");
        for (i, item) in items.iter().enumerate() {
            let line = lines.get(i).copied().or(key_line(manifest, "fields"));
            let parsed = serde_yaml::from_value::<FieldsFile>(item.clone())
                .map_err(|e| e.to_string())
                .and_then(|raw| {
                    Ok(FieldsSpec {
                        extension: extension.to_string(),
                        namespace: namespace.to_string(),
                        model: raw.model,
                        fields: fields_of(raw.fields)?,
                    })
                });
            match parsed {
                Ok(f) => out.fields.push(f),
                Err(e) => errors.push(at("fields.invalid", file, line, format!("fields: {e}"))),
            }
        }
    }
    (out, errors)
}

/// The change event of each model `declared` declares, as the extension's
/// own declared type with core's payload schema.
pub fn change_events(
    declared: &Declared,
    file: &str,
) -> Vec<crate::extension::extension_event_types::EventTypeDecl> {
    declared
        .models
        .iter()
        .filter_map(|m| {
            Some(crate::extension::extension_event_types::EventTypeDecl {
                event_type: m.change_event.clone()?,
                v: 1,
                schema: crate::models::change_event_schema(),
                summary: format!("A row of the shared model `{}` changed.", m.decl.id()),
                upcast: None,
                upcast_source: None,
                declared_at: format!("{file}: models: {}", m.decl.name),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_text(text: &str) -> (Declared, Vec<Diagnostic>) {
        let v: Value = serde_yaml::from_str(text).unwrap();
        parse(
            v.get("models"),
            v.get("fills"),
            v.get("fields"),
            "shelf",
            "shelf",
            "extension.yaml",
            text,
        )
    }

    /// A model, a fill and a fields entry parse into their specs, and the
    /// model's change event is a declared type with core's schema.
    #[test]
    fn the_three_keys_parse_into_specs() {
        let (d, errors) = parse_text(
            "models:\n  - name: book\n    resource: book\n    open: true\n    key: id\n    change_event: shelf.book.changed\n    columns:\n      id: text\n      body: { type: text, markdown: { pins: true } }\n      about: { type: uri, edge: book.about }\nfills:\n  - model: shelf/book\n    stores: [shelf-db]\n    fields: [{ name: rating, title: Rating, kind: number }]\nfields:\n  - model: other/thing\n    fields: [{ name: score, title: Score, kind: enum, values: [a, b] }]\n",
        );
        assert!(errors.is_empty(), "{errors:?}");
        let m = &d.models[0];
        assert_eq!(m.decl.resource, "shelf/book");
        assert!(m.open);
        assert_eq!(m.decl.store_param, "store");
        assert_eq!(d.fills[0].stores, vec!["shelf-db".to_string()]);
        assert_eq!(d.fields[0].fields[0].values.as_ref().unwrap().len(), 2);
        let events = change_events(&d, "extension.yaml");
        assert_eq!(events[0].event_type, "shelf.book.changed");
        assert_eq!(events[0].schema, crate::models::change_event_schema());
    }

    /// What is wrong is named at its line, and the rest still parses.
    #[test]
    fn a_bad_declaration_is_named_and_left_out() {
        let (d, errors) = parse_text(
            "models:\n  - name: book\n    resource: book\n    key: id\n    columns: { id: text, n: wide }\n  - name: ok\n    resource: ok\n    key: id\n    columns: { id: text }\n    change_event: other.changed\nfills:\n  - model: shelf/book\n    stores: [Bad Store]\n",
        );
        assert!(d.models.is_empty() && d.fills.is_empty(), "{d:?}");
        let text = format!("{errors:?}");
        assert!(text.contains("wide"), "{text}");
        assert!(text.contains("namespace"), "{text}");
        assert!(text.contains("Bad Store"), "{text}");
    }
}
