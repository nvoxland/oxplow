//! The provider read adapter (`.context/models.md` "Providers"): a
//! provider instance's `task.recorded` and `knowledge.page.recorded`
//! records become the same declarative write a collector's `models` output
//! makes — `models::apply_tx` into bundled's `oxplow/task_record` and
//! `oxplow/knowledge_record`, the store being the instance's id. It runs
//! beside today's projections (`tasks.project`, `knowledge.records`).
//!
//! With bundled's models not declared, or no fill for the instance (the
//! registry adds one for each running instance,
//! `ModelRegistry::with_provider_fills`), a record is not this consumer's.

use std::sync::Arc;

use async_trait::async_trait;
use oxplow_db::model_store::ModelRow;
use oxplow_domain::events::schema::{
    EventType as _, KnowledgePageRecord, KnowledgePageRecorded, KnowledgePageRecordedV3,
    TaskRecorded, TaskRecordedV3,
};
use oxplow_domain::tasks::Bucket;
use oxplow_domain::{DomainError, StoredEvent};
use serde_json::{json, Value};

use crate::models::{apply_tx, Cause, ModelOutput, Models, Writer};

const TASK_RECORD: &str = "oxplow/task_record";
const KNOWLEDGE_RECORD: &str = "oxplow/knowledge_record";

/// Writes a provider's records into the record models (`records.models`).
pub struct ModelRecords {
    pub models: Models,
    pub db: oxplow_db::Database,
    pub vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
}

impl ModelRecords {
    pub const NAME: &'static str = "records.models";
}

fn invalid(what: impl std::fmt::Display) -> DomainError {
    DomainError::Invalid(what.to_string())
}

#[async_trait]
impl crate::event_pump::AsyncEventConsumer for ModelRecords {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == TaskRecorded::TYPE || event_type == KnowledgePageRecorded::TYPE
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        let env = &event.envelope;
        let is_task = env.event_type == TaskRecorded::TYPE;
        let (model, instance, id) = if is_task {
            let TaskRecordedV3 { item } = serde_json::from_value(env.payload.clone())
                .map_err(|e| invalid(format!("task.recorded payload: {e}")))?;
            let Some((instance, id)) = oxplow_domain::uri::build::task_of_uri(&item.uri) else {
                return Ok(());
            };
            (TASK_RECORD, instance, id)
        } else {
            let KnowledgePageRecordedV3 { page } = serde_json::from_value(env.payload.clone())
                .map_err(|e| invalid(format!("knowledge.page.recorded payload: {e}")))?;
            let Some((instance, id)) = oxplow_domain::uri::build::knowledge_of_uri(&page.uri)
            else {
                return Ok(());
            };
            (KNOWLEDGE_RECORD, instance, id)
        };
        // Its own records, while its instance runs and bundled declares
        // the model.
        let registry = self.models.current();
        let Some(fill) = registry
            .fills
            .iter()
            .find(|f| f.model == model && f.stores.contains(&instance))
        else {
            return Ok(());
        };
        let extension = fill.extension.clone();
        let payload = env.payload.clone();
        let (at, cause, anchors, source) = (
            event.envelope.at.to_string(),
            env.id.clone(),
            env.anchors.clone(),
            oxplow_domain::uri::build::provider_source(&instance),
        );
        let vocabulary = self.vocabulary.clone();
        self.db
            .transaction(move |tx| {
                let vocabulary = vocabulary.current();
                let (upsert, delete) = if is_task {
                    let TaskRecordedV3 { item } = serde_json::from_value(payload.clone())
                        .map_err(|e| invalid(format!("task.recorded payload: {e}")))?;
                    if item.deleted {
                        (Vec::new(), vec![id.clone()])
                    } else {
                        let row = task_row(tx, &registry, &instance, &id, &item, &at)?;
                        (vec![row], Vec::new())
                    }
                } else {
                    let KnowledgePageRecordedV3 { page } = serde_json::from_value(payload.clone())
                        .map_err(|e| invalid(format!("knowledge.page.recorded payload: {e}")))?;
                    if page.deleted {
                        (Vec::new(), vec![id.clone()])
                    } else {
                        (vec![page_row(&id, &page)], Vec::new())
                    }
                };
                let output = ModelOutput {
                    model: model.to_string(),
                    store: instance.clone(),
                    upsert,
                    delete,
                    rename: Vec::new(),
                    snapshot: None,
                    confirm_removals: false,
                };
                let writer = Writer {
                    extension: &extension,
                    source: &source,
                    cause: Some(cause.clone()),
                    anchors: anchors.clone(),
                    why: Cause::Trigger,
                };
                let applied = apply_tx(tx, &registry, &vocabulary, &writer, &[output], &[])
                    .map_err(invalid)?;
                for problem in applied.problems {
                    tracing::warn!(%instance, %problem, "a provider record wasn't written");
                }
                Ok(())
            })
            .await
    }
}

/// The stage's title and bucket, as the instance declares its stages.
fn stage_of(
    conn: &rusqlite::Connection,
    instance: &str,
    stage: &str,
) -> Result<(String, Bucket), DomainError> {
    let storage = |e: rusqlite::Error| DomainError::Storage(e.to_string());
    let declared: Vec<(String, String, String)> = conn
        .prepare(
            "SELECT json_extract(s.value, '$.id'), json_extract(s.value, '$.title'),
                    json_extract(s.value, '$.bucket')
               FROM capability_provider c, json_each(c.stages_json) s
              WHERE c.capability = 'task_tracker' AND c.provider = ?1",
        )
        .and_then(|mut stmt| {
            stmt.query_map([instance], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .map_err(storage)?;
    declared
        .iter()
        .find(|(id, _, _)| id == stage)
        .and_then(|(_, title, bucket)| Some((title.clone(), Bucket::named(bucket)?)))
        .ok_or_else(|| {
            invalid(format!(
                "`{instance}` recorded stage `{stage}`, which it doesn't declare"
            ))
        })
}

/// What the row of `id` already holds in `column`, if it is there.
fn stored_time(
    conn: &rusqlite::Connection,
    registry: &crate::models::ModelRegistry,
    instance: &str,
    id: &str,
    column: &str,
) -> Option<String> {
    let table = registry.models.get(TASK_RECORD)?.decl.table();
    let uri = oxplow_domain::uri::build::task_uri(instance, id);
    conn.query_row(
        &format!("SELECT \"{column}\" FROM \"{table}\" WHERE uri = ?1 AND removed_at IS NULL"),
        [&uri],
        |r| r.get::<_, Option<String>>(0),
    )
    .ok()
    .flatten()
}

/// The time each stored comment of `id` was first seen with.
fn seen_comments(
    conn: &rusqlite::Connection,
    registry: &crate::models::ModelRegistry,
    instance: &str,
    id: &str,
) -> std::collections::BTreeMap<String, String> {
    let Some(decl) = registry.models.get(TASK_RECORD).map(|m| &m.decl) else {
        return Default::default();
    };
    let Some(child) = decl.children.iter().find(|c| c.name == "comments") else {
        return Default::default();
    };
    let uri = oxplow_domain::uri::build::task_uri(instance, id);
    let sql = format!(
        "SELECT \"id\", \"at\" FROM \"{}\" WHERE parent = ?1",
        decl.child_table(child)
    );
    conn.prepare(&sql)
        .and_then(|mut st| {
            st.query_map([&uri], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<_>>()
        })
        .unwrap_or_default()
}

fn task_row(
    conn: &rusqlite::Connection,
    registry: &crate::models::ModelRegistry,
    instance: &str,
    id: &str,
    item: &oxplow_domain::tasks::TaskRecord,
    at: &str,
) -> Result<ModelRow, DomainError> {
    let (stage_title, bucket) = stage_of(conn, instance, &item.stage)?;
    let spec = registry.models.get(TASK_RECORD);
    let fill = registry
        .fills
        .iter()
        .find(|f| f.model == TASK_RECORD && f.stores.iter().any(|s| s == instance));
    // When it first closed, as recorded; reopened, open again.
    let closed_at = bucket.is_closed().then(|| {
        stored_time(conn, registry, instance, id, "closed_at").unwrap_or_else(|| at.to_string())
    });
    let created_at =
        stored_time(conn, registry, instance, id, "created_at").unwrap_or_else(|| at.to_string());
    let mut row = ModelRow::new();
    row.insert("id".into(), json!(id));
    row.insert("title".into(), json!(item.title));
    row.insert("body".into(), json!(item.body));
    row.insert("state".into(), json!(bucket.as_str()));
    row.insert("stage".into(), json!(item.stage));
    row.insert("stage_title".into(), json!(stage_title));
    row.insert("parent".into(), json!(item.parent_uri));
    row.insert("closed_at".into(), json!(closed_at));
    row.insert("created_at".into(), json!(created_at));
    row.insert("updated_at".into(), json!(at));
    // Its own fields: the ones it declares.
    let mut fields = serde_json::Map::new();
    if let (Some(fill), Value::Object(native)) = (fill, &item.native) {
        for (name, value) in native {
            if !value.is_null() && fill.fields.iter().any(|f| &f.name == name) {
                fields.insert(name.clone(), value.clone());
            }
        }
    }
    row.insert("fields".into(), Value::Object(fields));
    // What the record states it states whole; what it leaves out, the
    // store keeps.
    if let Some(links) = &item.links {
        let types: Option<Vec<String>> = spec
            .and_then(|s| s.decl.children.iter().find(|c| c.name == "links"))
            .and_then(|c| c.columns.iter().find(|c| c.name == "type"))
            .and_then(|c| c.values.clone());
        let kept: Vec<Value> = links
            .iter()
            .filter(|l| {
                let known = oxplow_domain::uri::build::resource_of(&l.target).is_some()
                    && types.as_ref().is_none_or(|t| t.contains(&l.link_type));
                if !known {
                    tracing::warn!(item = %item.uri, target = %l.target, link_type = %l.link_type,
                        "a link the record model can't hold was dropped");
                }
                known
            })
            .map(|l| json!({ "to": l.target, "type": l.link_type }))
            .collect();
        row.insert("links".into(), Value::Array(kept));
    }
    if let Some(comments) = &item.comments {
        // A comment the record gives no time keeps the one it was first
        // seen with, so a restated record restates the same rows.
        let seen = seen_comments(conn, registry, instance, id);
        let list: Vec<Value> = comments
            .iter()
            .map(|c| {
                let when = c
                    .created_at
                    .clone()
                    .or_else(|| seen.get(&c.id).cloned())
                    .unwrap_or_else(|| at.to_string());
                json!({ "id": c.id, "author": c.author, "at": when, "body": c.body })
            })
            .collect();
        row.insert("comments".into(), Value::Array(list));
    }
    Ok(row)
}

fn page_row(id: &str, page: &KnowledgePageRecord) -> ModelRow {
    let mut row = ModelRow::new();
    row.insert("id".into(), json!(id));
    row.insert("title".into(), json!(page.title));
    row.insert("body".into(), json!(page.body));
    row.insert("kind".into(), json!(page.kind));
    row.insert("url".into(), json!(page.url));
    row.insert("author".into(), json!(page.author));
    row.insert("updated_at".into(), json!(page.updated_at));
    row.insert("refs".into(), json!(page.uris));
    row
}

/// Registered with the event pump beside the projections.
pub fn consumer(
    models: Models,
    db: oxplow_db::Database,
    vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
) -> Arc<ModelRecords> {
    Arc::new(ModelRecords {
        models,
        db,
        vocabulary,
    })
}
