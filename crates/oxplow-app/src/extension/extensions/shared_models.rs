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

use oxplow_db::model_store::{
    ChildDecl, ChildEdge, FieldDecl, Markdown, ModelColumn, ModelDecl, ModelType,
};
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
    /// The text columns search indexes as an item's title and body; the
    /// resource is searchable through a generated view unless it names its
    /// own `searchable:`.
    #[serde(default)]
    search: Option<SearchFile>,
    /// Rows that belong to one row of the model, by name.
    #[serde(default)]
    children: BTreeMap<String, ChildFile>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ChildFile {
    key: Vec<String>,
    #[schemars(with = "BTreeMap<String, serde_json::Value>")]
    columns: BTreeMap<String, Value>,
    #[serde(default)]
    edge: Option<ChildEdgeFile>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct ChildEdgeFile {
    to: String,
    #[serde(default)]
    type_from: Option<String>,
    #[serde(default)]
    ref_type: Option<String>,
    #[serde(default)]
    prefix: Option<String>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchFile {
    title: String,
    body: String,
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
    /// How its ids look in text, a regex matched whole: a loose id that
    /// matches is its store's item (a task model's stores).
    #[serde(default)]
    id_pattern: Option<String>,
    /// Its ids count in running text (`tsk42`), not only inside `[[…]]`
    /// (`[[some-page]]`): set with `id_pattern`.
    #[serde(default)]
    loose: bool,
    /// Its store's workflow, in order: `[{ id, title, bucket }]`, checked
    /// as a task tracker's stages are.
    #[serde(default)]
    stages: Vec<oxplow_domain::tasks::StageDecl>,
    #[serde(default)]
    fields: Vec<FieldFile>,
    /// Where its store keeps its rows as files: a file at
    /// `<folder>/<id><suffix>` is the row of that id (in the fill's first
    /// store), so an agent's read of the file is a read of the row.
    #[serde(default)]
    files: Option<FillFilesFile>,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FillFilesFile {
    /// A folder relative to the project root (`.oxplow/oxwiki`).
    folder: String,
    /// What follows the row's id in the file's name (`.md`).
    suffix: String,
}

#[derive(Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FieldsFile {
    model: String,
    fields: Vec<FieldFile>,
}

fn files_of(raw: FillFilesFile) -> Result<crate::models::FillFiles, String> {
    let folder = raw.folder.trim_end_matches('/').to_string();
    if !oxplow_domain::service::valid_path(&folder) {
        return Err(format!(
            "`files.folder` `{}` is a folder relative to the project root, with no `..`, `.` or empty parts",
            raw.folder
        ));
    }
    if !raw.suffix.starts_with('.') || raw.suffix.len() < 2 || raw.suffix.contains('/') {
        return Err(format!(
            "`files.suffix` `{}` is what follows the id in a file's name, such as `.md`",
            raw.suffix
        ));
    }
    Ok(crate::models::FillFiles {
        folder,
        suffix: raw.suffix,
    })
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

/// Every table and view name a model (and its children) generates.
fn generated_names(d: &oxplow_db::model_store::ModelDecl) -> Vec<(String, String)> {
    let mut out = vec![
        (d.table(), format!("model `{}`", d.name)),
        (d.view(), format!("model `{}`", d.name)),
        (d.view_all(), format!("model `{}`", d.name)),
        (format!("{}_field", d.view()), format!("model `{}`", d.name)),
        (d.search_view(), format!("model `{}`", d.name)),
    ];
    for c in &d.children {
        let what = format!("child `{}` of model `{}`", c.name, d.name);
        out.push((d.child_table(c), what.clone()));
        out.push((d.child_view(c), what));
    }
    out
}

/// The first generated name two models share, said naming both.
fn name_clash(
    a: &oxplow_db::model_store::ModelDecl,
    b: &oxplow_db::model_store::ModelDecl,
) -> Option<String> {
    let theirs = generated_names(a);
    generated_names(b).into_iter().find_map(|(name, what)| {
        theirs
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, other)| format!("{what} and {other} both generate `{name}`"))
    })
}

fn model_of(raw: ModelFile, extension: &str, namespace: &str) -> Result<ModelSpec, String> {
    if !word(&raw.name) {
        return Err(format!("`{}` isn't a model name (snake_case)", raw.name));
    }
    // Its generated views are `v_<ns>_<name>`, `…_all`, `…_field` and
    // `v_<ns>_search_<name>`: a model may not take one another's would.
    if raw.name.ends_with("_all") || raw.name.ends_with("_field") || raw.name.starts_with("search_")
    {
        return Err(format!(
            "`{}`: a model name may not end in `_all` or `_field` or start with `search_` \
             (its generated views use them)",
            raw.name
        ));
    }
    if format!("{namespace}/{}", raw.resource)
        .parse::<oxplow_domain::uri::ResourceName>()
        .is_err()
    {
        return Err(format!(
            "`resource: {}` isn't a resource name",
            raw.resource
        ));
    }
    if let Some(param) = &raw.store_param {
        if !word(param) || param == "id" {
            return Err(format!(
                "`store_param: {param}` must be a snake_case word other than `id`"
            ));
        }
    }
    let columns = columns_of(raw.columns, &raw.name)?;
    let children = raw
        .children
        .into_iter()
        .map(|(name, c)| child_of(name, c, &raw.name))
        .collect::<Result<Vec<_>, _>>()?;
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
        search: raw.search.map(|s| oxplow_db::model_store::ModelSearch {
            title: s.title,
            body: s.body,
        }),
        children,
    };
    // The store's rules for names, key and implicit columns.
    oxplow_db::model_store::check_decl(&decl).map_err(|e| e.to_string())?;
    Ok(ModelSpec {
        decl,
        open: raw.open,
        change_event: raw.change_event,
    })
}

fn child_of(name: String, raw: ChildFile, model: &str) -> Result<ChildDecl, String> {
    Ok(ChildDecl {
        columns: columns_of(raw.columns, model).map_err(|e| format!("child `{name}`: {e}"))?,
        name,
        key: raw.key,
        edge: raw.edge.map(|e| ChildEdge {
            to: e.to,
            type_from: e.type_from,
            ref_type: e.ref_type,
            prefix: e.prefix,
        }),
    })
}

/// The columns of a model (or a child) from its manifest map.
fn columns_of(
    raw_columns: BTreeMap<String, Value>,
    model: &str,
) -> Result<Vec<ModelColumn>, String> {
    let mut columns = Vec::new();
    for (name, v) in raw_columns {
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
        if let Some(edge) = &col.edge {
            let own = edge.strip_prefix(&format!("{model}.")).is_some_and(word);
            if !own {
                return Err(format!(
                    "column `{name}`: edge `{edge}` must be `{model}.<word>` (its model's own ref type)"
                ));
            }
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
    Ok(columns)
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
                Ok(m) => match out.models.iter().find_map(|o| name_clash(&o.decl, &m.decl)) {
                    Some(clash) => errors.push(
                        at("model.collision", file, line, format!("model: {clash}"))
                            .fix("rename one of the models or children"),
                    ),
                    None => out.models.push(m),
                },
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
                    if !raw.stages.is_empty() {
                        if let Some(problem) = oxplow_domain::tasks::stages_problem(&raw.stages) {
                            return Err(format!("`stages`: {problem}"));
                        }
                    }
                    if let Some(pattern) = &raw.id_pattern {
                        regex::Regex::new(&format!("^(?:{pattern})$"))
                            .map_err(|e| format!("`id_pattern` `{pattern}` isn't a regex: {e}"))?;
                    }
                    if raw.loose && raw.id_pattern.is_none() {
                        return Err(
                            "`loose` needs an `id_pattern` to say what a loose id looks like"
                                .into(),
                        );
                    }
                    Ok(FillSpec {
                        extension: extension.to_string(),
                        model: raw.model,
                        stores,
                        fields: fields_of(raw.fields)?,
                        id_pattern: raw.id_pattern,
                        loose: raw.loose,
                        stages: raw.stages,
                        files: raw.files.map(files_of).transpose()?,
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

    /// Names two models generate in common are refused at load, naming both.
    /// A fill's `stages` are its store's workflow, checked as a task
    /// tracker's are; bad stages name the problem and leave the fill out.
    #[test]
    fn a_fills_stages_are_its_stores_workflow() {
        let fill = |stages: &str| {
            format!(
                "fills:\n  - model: oxplow/task_record\n    stores: [t]\n    stages: {stages}\n"
            )
        };
        let (d, errors) = parse_text(&fill(
            "[{ id: todo, title: To do, bucket: todo }, { id: doing, title: Doing, bucket: in_progress }, { id: done, title: Done, bucket: done }]",
        ));
        assert!(errors.is_empty(), "{errors:?}");
        let ids: Vec<&str> = d.fills[0].stages.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["todo", "doing", "done"]);
        let (d, errors) = parse_text(&fill("[{ id: todo, title: To do, bucket: todo }]"));
        assert!(d.fills.is_empty());
        assert!(errors[0].message.contains("stages"), "{errors:?}");
    }

    #[test]
    fn colliding_generated_names_are_refused() {
        let child =
            "    children:\n      notes:\n        key: [id]\n        columns: { id: text }\n";
        let model = |name: &str, kids: bool| {
            format!(
                "  - name: {name}\n    resource: {name}\n    key: id\n    columns:\n      id: text\n{}",
                if kids { child } else { "" }
            )
        };
        for other in ["book_notes", "book__notes"] {
            let text = format!("models:\n{}{}", model("book", true), model(other, false));
            let (d, errors) = parse_text(&text);
            assert_eq!(d.models.len(), 1, "{other}");
            let said = errors[0].message.clone();
            assert!(said.contains("notes") && said.contains(other), "{said}");
        }
        let ok = format!(
            "models:\n{}{}",
            model("book", true),
            model("shelf_item", false)
        );
        assert!(parse_text(&ok).1.is_empty());
    }

    const TASK: &str = "models:\n  - name: task\n    resource: task\n    open: true\n    key: id\n    columns:\n      id: text\n    children:\n      links:\n        key: [to, type]\n        columns:\n          to: uri\n          type: { type: text, values: [blocks, relates_to] }\n        edge: { to: to, type_from: type, prefix: \"task_link:\" }\n      comments:\n        key: [id]\n        columns: { id: text, body: { type: text, markdown: {} } }\n";

    /// A model's children parse into `ModelDecl.children`, edge included.
    #[test]
    fn children_parse_into_the_decl() {
        let (d, errors) = parse_text(TASK);
        assert!(errors.is_empty(), "{errors:?}");
        let kids = &d.models[0].decl.children;
        assert_eq!(kids.len(), 2);
        let links = kids.iter().find(|c| c.name == "links").unwrap();
        assert_eq!(links.key, vec!["to".to_string(), "type".to_string()]);
        assert_eq!(links.columns.len(), 2);
        let edge = links.edge.as_ref().unwrap();
        assert_eq!(edge.to, "to");
        assert_eq!(edge.type_from.as_deref(), Some("type"));
        assert_eq!(edge.prefix.as_deref(), Some("task_link:"));
        let comments = kids.iter().find(|c| c.name == "comments").unwrap();
        assert!(comments.edge.is_none());
        assert!(comments.columns.iter().any(|c| c.markdown.is_some()));
    }

    /// A child's names, key and edge are checked; the rule for an edge's
    /// ref types keeps core's out.
    #[test]
    fn a_bad_child_is_named_and_left_out() {
        for (from, to, says) in [
            ("key: [to, type]", "key: [nope]", "key `nope`"),
            (
                "edge: { to: to,",
                "edge: { to: type,",
                "must be a uri column",
            ),
            (
                "type_from: type, prefix: \"task_link:\"",
                "prefix: \"task_link:\"",
                "exactly one",
            ),
            (
                "type_from: type, prefix: \"task_link:\"",
                "type_from: type, prefix: \"file_mention:\"",
                "prefix",
            ),
            (
                "type_from: type, prefix: \"task_link:\"",
                "type_from: type, prefix: \"task_link\"",
                "prefix",
            ),
            (
                "type_from: type, prefix: \"task_link:\"",
                "ref_type: file_mention",
                "ref_type",
            ),
            ("type_from: type,", "type_from: to,", "type_from"),
            ("      links:", "      Links:", "may take"),
        ] {
            let manifest = TASK.replacen(from, to, 1);
            let (d, errors) = parse_text(&manifest);
            assert!(d.models.is_empty(), "{to}");
            assert!(format!("{errors:?}").contains(says), "{to}: {errors:?}");
        }
        // A fixed ref type of the model's own is fine.
        let (d, errors) = parse_text(&TASK.replacen(
            "type_from: type, prefix: \"task_link:\"",
            "ref_type: task.link",
            1,
        ));
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            d.models[0].decl.children[1]
                .edge
                .as_ref()
                .unwrap()
                .ref_type
                .as_deref(),
            Some("task.link")
        );
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
        for (manifest, says) in [
            ("models:\n  - name: book\n    resource: Book\n    key: id\n    columns: { id: text }\n", "resource name"),
            ("models:\n  - name: book\n    resource: book\n    store_param: id\n    key: id\n    columns: { id: text }\n", "other than `id`"),
            ("models:\n  - name: book_all\n    resource: b\n    key: id\n    columns: { id: text }\n", "generated views"),
            ("models:\n  - name: b\n    resource: b\n    key: id\n    columns: { id: text, to: { type: uri, edge: file_pin } }\n", "own ref type"),
        ] {
            let (d, errors) = parse_text(manifest);
            assert!(d.models.is_empty(), "{manifest}");
            assert!(format!("{errors:?}").contains(says), "{manifest}: {errors:?}");
        }
    }
}
