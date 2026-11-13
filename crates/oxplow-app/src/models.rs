//! Shared models in the app (`.context/models.md`): which models the
//! enabled extensions declare, which stores and fields each may write, and
//! applying a collector's declarative `models` / `fields` output in its
//! run's transaction — the store write (`oxplow_db::model_store`) and one
//! change event per changed row, of the type the model's owner names.
//!
//! The change event's payload is the same for every model, so core gives
//! its schema ([`change_event_schema`]); the owner only names the type.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use oxplow_db::model_store::{
    write_fields_tx, write_tx, ChangeOp, FieldDecl, ModelChange, ModelDecl, ModelRow, ModelWrite,
};
use oxplow_db::EventCtx;
use oxplow_domain::events::{Anchors, Envelope, EventId};
use oxplow_domain::vocabulary::Vocabulary;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// A model an enabled extension declares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelSpec {
    pub decl: ModelDecl,
    /// Other extensions may fill it.
    pub open: bool,
    /// The event type logged per changed row.
    pub change_event: Option<String>,
}

/// What one extension may write into one model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FillSpec {
    pub extension: String,
    /// `<owner ns>/<model>`.
    pub model: String,
    pub stores: Vec<String>,
    /// The store's own fields.
    pub fields: Vec<FieldDecl>,
    /// How its items' ids look in text (a regex matched whole, `tsk[0-9]+`):
    /// a loose id that matches is its store's item (`fills.id_pattern`).
    #[serde(default)]
    pub id_pattern: Option<String>,
    /// Its ids count in running text (`tsk42`), not only inside `[[…]]`
    /// (`fills.loose`).
    #[serde(default)]
    pub loose: bool,
    /// Its store's workflow, in order (`fills.stages`): each stage's id, title and bucket.
    #[serde(default)]
    pub stages: Vec<oxplow_domain::tasks::StageDecl>,
    /// The files its store keeps its rows in (`fills.files`): an agent's read
    /// of one is a read of the row.
    #[serde(default)]
    pub files: Option<FillFiles>,
}

/// Where a store keeps its rows as files: a file at
/// `<folder>/<id><suffix>` (folder relative to the project root) is the row
/// of that id in the fill's first store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FillFiles {
    pub folder: String,
    pub suffix: String,
}

/// Fields an extension declares on a model it doesn't fill.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldsSpec {
    pub extension: String,
    pub namespace: String,
    pub model: String,
    pub fields: Vec<FieldDecl>,
}

/// Every enabled extension's models, fills and field declarations.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelRegistry {
    pub models: BTreeMap<String, ModelSpec>,
    pub fills: Vec<FillSpec>,
    pub fields: Vec<FieldsSpec>,
}

/// The registry the app reads, swapped whole when what runs changes.
#[derive(Clone, Default)]
pub struct Models(Arc<RwLock<Arc<ModelRegistry>>>);

impl Models {
    pub fn current(&self) -> Arc<ModelRegistry> {
        self.0.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn swap(&self, registry: ModelRegistry) {
        *self.0.write().unwrap_or_else(|e| e.into_inner()) = Arc::new(registry);
    }
}

impl ModelRegistry {
    /// The enabled extensions' models, fills and field declarations. A fill
    /// of a model no enabled extension declares, of one that isn't open, or
    /// with a store id another extension already writes is left out (its
    /// writes are refused, naming the reason).
    pub fn of(extensions: &[crate::extension::extensions::Extension]) -> Self {
        let mut out = Self::default();
        for e in extensions.iter().filter(|e| e.enabled) {
            for m in &e.shared_models.models {
                out.models.entry(m.decl.id()).or_insert_with(|| m.clone());
            }
        }
        for e in extensions.iter().filter(|e| e.enabled) {
            for f in &e.shared_models.fills {
                let Some(spec) = out.models.get(&f.model) else {
                    continue;
                };
                if !spec.open && spec.decl.extension != f.extension {
                    continue;
                }
                let taken = out.fills.iter().any(|o| {
                    o.model == f.model
                        && o.extension != f.extension
                        && o.stores.iter().any(|s| f.stores.contains(s))
                });
                if !taken {
                    out.fills.push(f.clone());
                }
            }
            out.fields.extend(
                e.shared_models
                    .fields
                    .iter()
                    .filter(|f| out.models.contains_key(&f.model))
                    .cloned(),
            );
        }
        out
    }
}

impl ModelRegistry {
    /// The row whose file `rel` (a path from the project root) is, as a
    /// fill's `files` says: `(model id, row uri)`.
    pub fn row_of_file(&self, rel: &str) -> Option<(String, String)> {
        for fill in &self.fills {
            let Some(files) = &fill.files else { continue };
            let Some(rest) = rel
                .strip_prefix(files.folder.trim_end_matches('/'))
                .and_then(|r| r.strip_prefix('/'))
            else {
                continue;
            };
            let Some(id) = rest.strip_suffix(files.suffix.as_str()) else {
                continue;
            };
            if id.is_empty() || id.contains('/') {
                continue;
            }
            let (Some(store), Some(spec)) = (fill.stores.first(), self.models.get(&fill.model))
            else {
                continue;
            };
            let uri = oxplow_domain::uri::build::of(
                &spec.decl.resource,
                &[(&spec.decl.key, id), (&spec.decl.store_param, store)],
            );
            return Some((fill.model.clone(), uri));
        }
        None
    }

    /// Each store of each model, with how its records' ids look in text, as
    /// its fill says (`fills.id_pattern`, `fills.loose`) and its model's
    /// resource names its URIs: what makes `[[<store>:<id>]]`, a bare
    /// `[[id]]` the pattern matches, and a loose id in running text, one of
    /// its records. In fill order: the first told of wins an id two match.
    pub fn id_patterns(&self) -> Vec<oxplow_domain::uri::resources::RecordIds> {
        self.fills
            .iter()
            .filter_map(|f| self.models.get(&f.model).map(|m| (f, m)))
            .flat_map(|(f, m)| {
                f.stores
                    .iter()
                    .map(move |store| oxplow_domain::uri::resources::RecordIds {
                        resource: m.decl.resource.clone(),
                        store: store.clone(),
                        store_param: m.decl.store_param.clone(),
                        key: m.decl.key.clone(),
                        pattern: f.id_pattern.clone(),
                        loose: f.loose,
                    })
            })
            .collect()
    }
}

impl ModelRegistry {
    /// What the enabled extensions' declarations can't both have, as errors
    /// on the later extension: a store id two extensions write into one
    /// model, and a store id that is another extension's field namespace on
    /// it (the store's restatement of its own fields would remove theirs).
    pub fn conflicts(
        extensions: &[crate::extension::extensions::Extension],
    ) -> crate::vocabulary_reactor::VocabularyErrors {
        use oxplow_domain::diagnostic::Diagnostic;
        let mut errors = crate::vocabulary_reactor::VocabularyErrors::new();
        let mut claimed: BTreeMap<(String, String), String> = BTreeMap::new();
        let enabled: Vec<_> = extensions.iter().filter(|e| e.enabled).collect();
        for e in &enabled {
            for f in &e.shared_models.fills {
                for store in &f.stores {
                    match claimed.get(&(f.model.clone(), store.clone())) {
                        Some(owner) if owner != &e.name => {
                            errors.entry(e.name.clone()).or_default().push(
                                Diagnostic::error(
                                    "model.store_taken",
                                    format!(
                                        "`{}` fills `{}` as store `{store}`, which `{owner}` already writes",
                                        e.name, f.model
                                    ),
                                )
                                .fix("name a store id of its own under `fills:`"),
                            );
                        }
                        _ => {
                            claimed.insert((f.model.clone(), store.clone()), e.name.clone());
                        }
                    }
                }
            }
        }
        for e in &enabled {
            for f in &e.shared_models.fields {
                if let Some(owner) = claimed.get(&(f.model.clone(), f.namespace.clone())) {
                    if owner != &e.name {
                        errors.entry(e.name.clone()).or_default().push(Diagnostic::error(
                            "model.namespace_taken",
                            format!(
                                "`{}`'s fields on `{}` would be kept as `{}`, the store id `{owner}` writes",
                                e.name, f.model, f.namespace
                            ),
                        ));
                    }
                }
            }
        }
        errors
    }

    /// The stores whose rows `model`'s views show: the enabled fillers'.
    pub fn live_stores(&self, model: &str) -> Vec<String> {
        self.fills
            .iter()
            .filter(|f| f.model == model)
            .flat_map(|f| f.stores.iter().cloned())
            .collect()
    }

    /// Every declared field as `(model, namespace, extension, field)`: a
    /// store's under each of its store ids, another extension's under its
    /// namespace (`v_model_field_decl`).
    /// Each live store of each model: `(model, store, extension, provider,
    /// id_pattern)`.
    pub fn store_decls(&self) -> Vec<oxplow_db::model_store::StoreDecl> {
        self.fills
            .iter()
            .filter_map(|f| self.models.get(&f.model).map(|m| (f, m)))
            .flat_map(|(f, m)| {
                f.stores
                    .iter()
                    .map(move |store| oxplow_db::model_store::StoreDecl {
                        model: f.model.clone(),
                        store: store.clone(),
                        extension: f.extension.clone(),
                        resource: m.decl.resource.clone(),
                        store_param: m.decl.store_param.clone(),
                        key: m.decl.key.clone(),
                        id_pattern: f.id_pattern.clone(),
                        loose: f.loose,
                    })
            })
            .collect()
    }

    /// Each filled store's stages: `(model, store, extension, stages)`.
    pub fn stage_decls(
        &self,
    ) -> Vec<(String, String, String, Vec<oxplow_domain::tasks::StageDecl>)> {
        self.fills
            .iter()
            .filter(|f| !f.stages.is_empty())
            .flat_map(|f| {
                f.stores.iter().map(move |store| {
                    (
                        f.model.clone(),
                        store.clone(),
                        f.extension.clone(),
                        f.stages.clone(),
                    )
                })
            })
            .collect()
    }

    pub fn field_decls(&self) -> Vec<(String, String, String, FieldDecl)> {
        let mut out = Vec::new();
        for fill in &self.fills {
            for store in &fill.stores {
                for f in &fill.fields {
                    out.push((
                        fill.model.clone(),
                        store.clone(),
                        fill.extension.clone(),
                        f.clone(),
                    ));
                }
            }
        }
        for decl in &self.fields {
            for f in &decl.fields {
                out.push((
                    decl.model.clone(),
                    decl.namespace.clone(),
                    decl.extension.clone(),
                    f.clone(),
                ));
            }
        }
        out
    }
}

/// The code that decides a model row's edges: the store and the shared
/// projections. A model's edges are repaired when it changes.
const EDGE_CODE: &[&str] = &[
    include_str!("../../oxplow-db/src/model_store.rs"),
    include_str!("../../oxplow-db/src/page_ref_projections.rs"),
];

/// Repair each model's edges (`model_store::repair_edges_tx`) whose last
/// repair was made by other edge code, another declaration of it or
/// another vocabulary (which ids resolve), recorded in `asset_state` as
/// `model_repair:<model>`; a repair rewrites only what drifted.
pub fn repair_edges_if_needed_tx(
    conn: &rusqlite::Connection,
    registry: &ModelRegistry,
    kinds: &oxplow_domain::uri::resources::ResourceRegistry,
) -> Result<(), oxplow_domain::DomainError> {
    use rusqlite::OptionalExtension;
    for (id, spec) in &registry.models {
        let mut hasher = xxhash_rust::xxh3::Xxh3::new();
        for source in EDGE_CODE {
            hasher.update(source.as_bytes());
        }
        hasher.update(
            serde_json::to_string(&spec.decl)
                .unwrap_or_default()
                .as_bytes(),
        );
        hasher.update(format!("{kinds:?}").as_bytes());
        let key = format!("{:016x}", hasher.digest());
        let asset = format!("model_repair:{id}");
        let stored: Option<String> = conn
            .query_row(
                "SELECT built_from FROM asset_state WHERE asset = ?1",
                [&asset],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(oxplow_db::map_sql_err)?
            .flatten();
        if stored.as_deref() == Some(key.as_str()) {
            continue;
        }
        let started = std::time::Instant::now();
        oxplow_db::model_store::repair_edges_tx(conn, &spec.decl, kinds)?;
        conn.execute(
            "INSERT INTO asset_state (asset, computed_at, events_to, elapsed_ms, built_from)
             VALUES (?1, ?2, (SELECT coalesce(max(seq), 0) FROM event_log), ?3, ?4)
             ON CONFLICT (asset) DO UPDATE SET
                computed_at = excluded.computed_at, events_to = excluded.events_to,
                elapsed_ms = excluded.elapsed_ms, built_from = excluded.built_from",
            rusqlite::params![
                asset,
                oxplow_domain::Timestamp::now().to_string(),
                started.elapsed().as_millis() as i64,
                key
            ],
        )
        .map_err(oxplow_db::map_sql_err)?;
    }
    Ok(())
}

/// The JSON Schema of every model's change event.
pub fn change_event_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "type": "object",
        "additionalProperties": false,
        "required": ["uri", "store", "id", "op", "changed", "cause"],
        "properties": {
            "uri": {"type": "string"},
            "store": {"type": "string"},
            "id": {"type": "string"},
            "op": {"enum": ["upserted", "removed", "renamed"]},
            "changed": {"type": "array", "items": {"type": "string"}},
            "before": {"type": "object"},
            "after": {"type": "object"},
            "cause": {"enum": ["trigger", "reconcile", "fields"]},
            "renamed_from": {"type": "string"}
        }
    })
}

/// One model write in a collector's output.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelOutput {
    pub model: String,
    pub store: String,
    #[serde(default)]
    pub upsert: Vec<ModelRow>,
    #[serde(default)]
    pub delete: Vec<String>,
    #[serde(default)]
    pub rename: Vec<Rename>,
    #[serde(default)]
    pub snapshot: Option<Vec<ModelRow>>,
    #[serde(default)]
    pub confirm_removals: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rename {
    pub from: String,
    pub to: String,
}

/// One extension's field values on an item, in a collector's output.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldsOutput {
    pub model: String,
    pub uri: String,
    pub values: serde_json::Map<String, Value>,
}

/// What applying an output did, per model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Applied {
    /// `model` → `{upserted, removed, renamed, unchanged, quarantined}`.
    pub counts: BTreeMap<String, BTreeMap<&'static str, usize>>,
    /// Rows skipped, each named.
    pub problems: Vec<String>,
}

/// Why a write ran: a trigger, or a `start` reconcile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cause {
    Trigger,
    Reconcile,
}

/// Who writes, and what its change events are caused by.
pub struct Writer<'a> {
    pub extension: &'a str,
    /// The events' source (the collector's URI).
    pub source: &'a str,
    pub cause: Option<EventId>,
    pub anchors: Anchors,
    pub why: Cause,
}

fn event_of(
    vocabulary: &Vocabulary,
    spec: &ModelSpec,
    writer: &Writer<'_>,
    change: &ModelChange,
    cause: &str,
) -> Result<Option<Envelope>, String> {
    let Some(event_type) = &spec.change_event else {
        return Ok(None);
    };
    let Some(v) = vocabulary.latest(event_type) else {
        return Err(format!(
            "model `{}`: its change event `{event_type}` isn't declared",
            spec.decl.id()
        ));
    };
    let mut payload = json!({
        "uri": change.uri, "store": change.store, "id": change.id,
        "op": change.op.as_str(), "changed": change.changed, "cause": cause,
    });
    if !change.before.is_empty() {
        payload["before"] = Value::Object(change.before.clone());
    }
    if !change.after.is_empty() {
        payload["after"] = Value::Object(change.after.clone());
    }
    if let Some(from) = &change.renamed_from {
        payload["renamed_from"] = json!(from);
    }
    let mut env = Envelope::new(event_type, v, writer.source, payload)
        .map_err(|e| e.to_string())?
        .with_anchors(writer.anchors.clone())
        .with_subject([change.uri.clone()]);
    env.cause = writer.cause.clone();
    Ok(Some(env))
}

/// Apply `writer`'s `models` and `fields` output in `conn` (its run's
/// transaction): each write checked against what the writer may fill, the
/// rows written by diff, one change event per changed row. A write the
/// writer may not make, or a snapshot held by the removal guard, fails the
/// whole output (nothing of it lands); a bad row is only skipped.
pub fn apply_tx(
    conn: &rusqlite::Connection,
    registry: &ModelRegistry,
    vocabulary: &Vocabulary,
    writer: &Writer<'_>,
    models: &[ModelOutput],
    fields: &[FieldsOutput],
) -> Result<Applied, String> {
    let kinds = &vocabulary.resources;
    let mut out = Applied::default();
    let ctx = EventCtx {
        vocabulary,
        source: writer.source.to_string(),
        cause: writer.cause.clone(),
    };
    let log = |env: Option<Envelope>| -> Result<(), String> {
        if let Some(env) = env {
            ctx.append(conn, &env).map_err(|e| e.to_string())?;
        }
        Ok(())
    };
    for m in models {
        let spec = registry
            .models
            .get(&m.model)
            .ok_or_else(|| format!("no model `{}` is declared", m.model))?;
        let fill = registry
            .fills
            .iter()
            .find(|f| f.extension == writer.extension && f.model == m.model)
            .filter(|_| spec.open || spec.decl.extension == writer.extension)
            .ok_or_else(|| {
                format!(
                    "`{}` doesn't fill `{}`: add it to its fills:",
                    writer.extension, m.model
                )
            })?;
        if !fill.stores.contains(&m.store) {
            return Err(format!(
                "`{}` may write `{}` only as {}, not `{}`",
                writer.extension,
                m.model,
                fill.stores.join(", "),
                m.store
            ));
        }
        let write = match &m.snapshot {
            Some(rows) => ModelWrite::Snapshot {
                rows: rows.clone(),
                confirm_removals: m.confirm_removals,
            },
            None => ModelWrite::Upsert {
                rows: m.upsert.clone(),
                delete: m.delete.clone(),
                rename: m
                    .rename
                    .iter()
                    .map(|r| (r.from.clone(), r.to.clone()))
                    .collect(),
            },
        };
        let written = write_tx(
            conn,
            &spec.decl,
            kinds,
            writer.source,
            &m.store,
            &fill.fields,
            &write,
        )
        .map_err(|e| e.to_string())?;
        let counts = out.counts.entry(m.model.clone()).or_default();
        // A snapshot states the whole store: catching up, whatever ran it.
        let cause = match (writer.why, m.snapshot.is_some()) {
            (Cause::Trigger, false) => "trigger",
            _ => "reconcile",
        };
        for change in &written.changes {
            let key = match change.op {
                ChangeOp::Upserted => "upserted",
                ChangeOp::Removed => "removed",
                ChangeOp::Renamed => "renamed",
            };
            *counts.entry(key).or_default() += 1;
            log(event_of(vocabulary, spec, writer, change, cause)?)?;
        }
        *counts.entry("unchanged").or_default() += written.unchanged;
        *counts.entry("quarantined").or_default() += written.quarantined.len();
        out.problems
            .extend(written.quarantined.iter().map(|q| q.message.clone()));
    }
    for f in fields {
        let spec = registry
            .models
            .get(&f.model)
            .ok_or_else(|| format!("no model `{}` is declared", f.model))?;
        let decls = registry
            .fields
            .iter()
            .find(|d| d.extension == writer.extension && d.model == f.model)
            .ok_or_else(|| {
                format!(
                    "`{}` declares no fields on `{}`: add them to its fields:",
                    writer.extension, f.model
                )
            })?;
        if let Some(change) = write_fields_tx(
            conn,
            &spec.decl,
            &decls.namespace,
            &decls.fields,
            &f.uri,
            &f.values,
        )
        .map_err(|e| e.to_string())?
        {
            *out.counts
                .entry(f.model.clone())
                .or_default()
                .entry("fields")
                .or_default() += 1;
            log(event_of(vocabulary, spec, writer, &change, "fields")?)?;
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "models_e2e_tests.rs"]
mod e2e_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::model_store::{ensure_model_tx, ChildDecl, Markdown, ModelColumn, ModelType};

    const EVENT: &str = "shelf.book.changed";

    fn registry() -> ModelRegistry {
        let mut body = ModelColumn::new("body", ModelType::Text);
        body.markdown = Some(Markdown { pins: false });
        let decl = ModelDecl {
            extension: "shelf".into(),
            namespace: "shelf".into(),
            name: "book".into(),
            resource: "shelf/book".into(),
            key: "id".into(),
            store_param: "store".into(),
            search: None,
            columns: vec![
                ModelColumn::new("id", ModelType::Text),
                ModelColumn::new("title", ModelType::Text),
                body,
            ],
            children: vec![ChildDecl {
                name: "notes".into(),
                key: vec!["id".into()],
                columns: vec![
                    ModelColumn::new("id", ModelType::Text),
                    ModelColumn::new("text", ModelType::Text),
                ],
                edge: None,
            }],
        };
        let mut r = ModelRegistry::default();
        r.models.insert(
            "shelf/book".into(),
            ModelSpec {
                decl,
                open: true,
                change_event: Some(EVENT.into()),
            },
        );
        r.fills.push(FillSpec {
            extension: "shelf-folder".into(),
            model: "shelf/book".into(),
            stores: vec!["shelf-folder".into()],
            fields: vec![],
            id_pattern: None,
            loose: false,
            stages: vec![],
            files: None,
        });
        r.fields.push(FieldsSpec {
            extension: "shelf-score".into(),
            namespace: "shelf-score".into(),
            model: "shelf/book".into(),
            fields: vec![FieldDecl {
                name: "score".into(),
                title: "Score".into(),
                kind: "number".into(),
                values: None,
                read_only: false,
            }],
        });
        r
    }

    fn vocabulary() -> Vocabulary {
        use oxplow_domain::uri::resources::{ParamDecl, ResourceDecl};
        let mut book = ResourceDecl::keyed("shelf/book", "id");
        book.params.insert(
            "store".into(),
            ParamDecl {
                required: true,
                pattern: None,
            },
        );
        let mut v = Vocabulary::core();
        v.resources.register(book).unwrap();
        v.with_declared(
            "shelf",
            "shelf",
            [oxplow_domain::events::schema::DeclaredEventType {
                event_type: EVENT.into(),
                v: 1,
                schema: change_event_schema(),
                summary: "A book changed.".into(),
                upcast: None,
            }],
        )
    }

    fn writer(extension: &'static str, why: Cause) -> Writer<'static> {
        Writer {
            extension,
            source: "oxplow:oxplow/collector?id=books&owner=shelf-folder",
            cause: None,
            anchors: Anchors::default(),
            why,
        }
    }

    /// A task model's stores that declare an id pattern recognize their
    /// ids; another model's fills, and a fill with none, don't.
    /// A fill's `files` names the file a store keeps each row in: a file at
    /// `<folder>/<id><suffix>` is that row of the fill's first store, and
    /// nothing else is.
    #[test]
    fn a_fills_files_name_the_row_each_file_is() {
        let mut r = registry();
        r.fills[0].files = Some(FillFiles {
            folder: ".oxplow/shelves".into(),
            suffix: ".md".into(),
        });
        let store = r.fills[0].stores[0].clone();
        let (model, row) = r
            .row_of_file(".oxplow/shelves/dune.md")
            .expect("a row's file");
        assert_eq!(model, r.fills[0].model);
        assert!(
            row.contains("id=dune") && row.contains(&format!("={store}")),
            "{row}"
        );
        for not in [
            ".oxplow/shelves/dune.txt",
            ".oxplow/shelves/.md",
            ".oxplow/shelves/sub/dune.md",
            ".oxplow/shelvesx/dune.md",
            "dune.md",
        ] {
            assert_eq!(r.row_of_file(not), None, "{not}");
        }
        r.fills[0].files = None;
        assert_eq!(r.row_of_file(".oxplow/shelves/dune.md"), None);
    }

    #[test]
    fn every_store_names_its_id_patterns_by_its_models_resource() {
        use oxplow_domain::uri::resources::RecordIds;
        let mut r = registry();
        r.fills[0].id_pattern = Some("bk[0-9]+".into());
        let book = r.models["shelf/book"].decl.clone();
        assert_eq!(
            r.id_patterns(),
            vec![RecordIds {
                resource: book.resource.clone(),
                store: r.fills[0].stores[0].clone(),
                store_param: book.store_param.clone(),
                key: book.key.clone(),
                pattern: Some("bk[0-9]+".into()),
                loose: false,
            }]
        );
        // A second model, filled loosely by one store and quietly by another.
        let mut other = r.models["shelf/book"].clone();
        other.decl.resource = "acme/ticket".into();
        other.decl.store_param = "queue".into();
        r.models.insert("x/ticket".into(), other);
        let fill = |store: &str, pattern: Option<&str>, loose| FillSpec {
            extension: store.into(),
            model: "x/ticket".into(),
            stores: vec![store.into()],
            fields: vec![],
            id_pattern: pattern.map(str::to_string),
            loose,
            stages: vec![],
            files: None,
        };
        r.fills.push(fill("tracker", Some("tsk[0-9]+"), true));
        r.fills.push(fill("quiet", None, false));
        let all = r.id_patterns();
        assert_eq!(all.len(), 3);
        assert_eq!(
            (&all[1].resource[..], &all[1].store_param[..], all[1].loose),
            ("acme/ticket", "queue", true)
        );
        assert_eq!(all[2].pattern, None);
    }

    fn output(v: Value) -> ModelOutput {
        serde_json::from_value(v).unwrap()
    }

    async fn apply(
        db: &oxplow_db::Database,
        extension: &'static str,
        why: Cause,
        models: Vec<ModelOutput>,
        fields: Vec<FieldsOutput>,
    ) -> Result<Applied, String> {
        db.transaction_or(move |tx| {
            for spec in registry().models.values() {
                ensure_model_tx(tx, &spec.decl).map_err(oxplow_db::TxError::Storage)?;
            }
            apply_tx(
                tx,
                &registry(),
                &vocabulary(),
                &writer(extension, why),
                &models,
                &fields,
            )
            .map_err(oxplow_db::TxError::Aborted)
        })
        .await
        .map_err(|e| match e {
            oxplow_db::TxError::Aborted(m) => m,
            other => format!("{other:?}"),
        })
    }

    async fn changes(db: &oxplow_db::Database) -> Vec<Value> {
        db.read(|tx| {
            let mut st = tx
                .prepare("SELECT payload FROM event_log WHERE type = ?1 ORDER BY seq")
                .map_err(oxplow_db::map_sql_err)?;
            let out = st
                .query_map([EVENT], |r| r.get::<_, String>(0))
                .map_err(oxplow_db::map_sql_err)?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(oxplow_db::map_sql_err)?;
            Ok(out
                .into_iter()
                .map(|p| serde_json::from_str(&p).unwrap())
                .collect())
        })
        .await
        .unwrap()
    }

    /// A store's snapshot writes its rows and logs one change event per
    /// changed row, with its cause; the same snapshot again logs none.
    #[tokio::test]
    async fn a_snapshot_logs_one_change_event_per_changed_row() {
        let db = oxplow_db::Database::in_memory();
        let snap = output(json!({"model": "shelf/book", "store": "shelf-folder",
            "snapshot": [{"id": "dune", "title": "Dune"}, {"id": "emma", "title": "Emma"}]}));
        let a = apply(
            &db,
            "shelf-folder",
            Cause::Reconcile,
            vec![snap.clone()],
            vec![],
        )
        .await
        .unwrap();
        assert_eq!(a.counts["shelf/book"]["upserted"], 2);
        let logged = changes(&db).await;
        assert_eq!(logged.len(), 2);
        assert_eq!(logged[0]["cause"], "reconcile");
        assert_eq!(
            logged[0]["uri"],
            "oxplow:shelf/book?id=dune&store=shelf-folder"
        );
        apply(&db, "shelf-folder", Cause::Reconcile, vec![snap], vec![])
            .await
            .unwrap();
        assert_eq!(
            changes(&db).await.len(),
            2,
            "an unchanged snapshot logs nothing"
        );
    }

    /// A snapshot is a reconcile whatever started the run: it states the
    /// whole store, so its changes are catching up, not "this happened now".
    #[tokio::test]
    async fn a_snapshot_is_a_reconcile_whatever_started_the_run() {
        let db = oxplow_db::Database::in_memory();
        let snap = output(json!({"model": "shelf/book", "store": "shelf-folder",
            "snapshot": [{"id": "dune", "title": "Dune"}]}));
        apply(&db, "shelf-folder", Cause::Trigger, vec![snap], vec![])
            .await
            .unwrap();
        assert_eq!(changes(&db).await[0]["cause"], "reconcile");
    }

    /// A change to a row's children is the row's change, naming the child.
    #[tokio::test]
    async fn a_changed_child_set_is_named_in_the_change_event() {
        let db = oxplow_db::Database::in_memory();
        let write = |notes: Value| {
            output(json!({"model": "shelf/book", "store": "shelf-folder",
                "upsert": [{"id": "dune", "title": "Dune", "notes": notes}]}))
        };
        apply(
            &db,
            "shelf-folder",
            Cause::Trigger,
            vec![write(json!([{"id": "n1", "text": "good"}]))],
            vec![],
        )
        .await
        .unwrap();
        apply(
            &db,
            "shelf-folder",
            Cause::Trigger,
            vec![write(json!([{"id": "n1", "text": "great"}]))],
            vec![],
        )
        .await
        .unwrap();
        let logged = changes(&db).await;
        assert_eq!(logged.len(), 2);
        assert_eq!(logged[1]["changed"], json!(["notes"]));
        let text: String = db
            .read(|tx| {
                tx.query_row("SELECT text FROM v_shelf_book_notes", [], |r| r.get(0))
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(text, "great");
    }

    /// Only a filler may write a model, and only as its own stores.
    #[tokio::test]
    async fn a_write_outside_its_fills_is_refused() {
        let db = oxplow_db::Database::in_memory();
        let err = apply(
            &db,
            "shelf-score",
            Cause::Trigger,
            vec![output(
                json!({"model": "shelf/book", "store": "shelf-score", "upsert": [{"id": "x"}]}),
            )],
            vec![],
        )
        .await
        .unwrap_err();
        assert!(err.contains("fills"), "{err}");
        let err = apply(
            &db,
            "shelf-folder",
            Cause::Trigger,
            vec![output(
                json!({"model": "shelf/book", "store": "someone-else", "upsert": [{"id": "x"}]}),
            )],
            vec![],
        )
        .await
        .unwrap_err();
        assert!(err.contains("someone-else"), "{err}");
    }

    /// Another extension's fields on a store's item land under its own
    /// namespace and log a change with cause `fields`.
    #[tokio::test]
    async fn another_extensions_fields_log_a_fields_change() {
        let db = oxplow_db::Database::in_memory();
        apply(
            &db,
            "shelf-folder",
            Cause::Trigger,
            vec![output(json!({"model": "shelf/book", "store": "shelf-folder", "upsert": [{"id": "dune", "title": "Dune"}]}))],
            vec![],
        )
        .await
        .unwrap();
        let f: FieldsOutput = serde_json::from_value(json!({"model": "shelf/book",
            "uri": "oxplow:shelf/book?id=dune&store=shelf-folder", "values": {"score": 9}}))
        .unwrap();
        apply(&db, "shelf-score", Cause::Trigger, vec![], vec![f])
            .await
            .unwrap();
        let logged = changes(&db).await;
        assert_eq!(logged.last().unwrap()["cause"], "fields");
        let fields: String = db
            .read(|tx| {
                tx.query_row("SELECT fields FROM v_shelf_book", [], |r| r.get(0))
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&fields).unwrap(),
            json!({"shelf-score": {"score": 9}})
        );
    }

    /// The edge repair runs for a model once per edge code, declaration and
    /// vocabulary: recorded, it doesn't run again until one of them changes.
    #[tokio::test]
    async fn the_edge_repair_runs_once_per_key() {
        let db = oxplow_db::Database::in_memory();
        let count = |db: oxplow_db::Database| async move {
            db.read(|tx| {
                tx.query_row(
                    "SELECT count(*) FROM asset_state WHERE asset = 'model_repair:shelf/book'",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap()
        };
        let computed = |db: oxplow_db::Database| async move {
            db.read(|tx| {
                tx.query_row(
                    "SELECT built_from FROM asset_state WHERE asset = 'model_repair:shelf/book'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap()
        };
        let repair = |db: oxplow_db::Database, registry: ModelRegistry| async move {
            db.transaction(move |tx| {
                for spec in registry.models.values() {
                    ensure_model_tx(tx, &spec.decl)?;
                }
                repair_edges_if_needed_tx(tx, &registry, &vocabulary().resources)
            })
            .await
            .unwrap()
        };
        repair(db.clone(), registry()).await;
        assert_eq!(count(db.clone()).await, 1);
        let first = computed(db.clone()).await;
        repair(db.clone(), registry()).await;
        assert_eq!(
            computed(db.clone()).await,
            first,
            "the same key repairs nothing"
        );
        let mut changed = registry();
        changed
            .models
            .get_mut("shelf/book")
            .unwrap()
            .decl
            .columns
            .push(oxplow_db::model_store::ModelColumn::new(
                "note",
                oxplow_db::model_store::ModelType::Text,
            ));
        repair(db.clone(), changed).await;
        assert_ne!(
            computed(db.clone()).await,
            first,
            "another declaration repairs again"
        );
    }
}
