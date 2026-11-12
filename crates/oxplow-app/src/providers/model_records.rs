//! The provider read adapter (`.context/models.md` "Providers"): a
//! provider instance's `knowledge.page.recorded` records become the same
//! declarative write a collector's `models` output makes —
//! `models::apply_tx` into bundled's `oxplow/knowledge_record`, the store
//! being the instance's id. It runs beside `knowledge.records`. (Knowledge
//! providers go with the `knowledge` capability.)
//!
//! With bundled's models not declared, or no fill for the instance (the
//! registry adds one for each running instance,
//! `ModelRegistry::with_provider_fills`), a record is not this consumer's.

use std::sync::Arc;

use async_trait::async_trait;
use oxplow_db::model_store::ModelRow;
use oxplow_domain::events::schema::{
    EventType as _, KnowledgePageRecord, KnowledgePageRecorded, KnowledgePageRecordedV3,
};
use oxplow_domain::{DomainError, StoredEvent};
use serde_json::json;

use crate::models::{apply_tx, Cause, ModelOutput, Models, Writer};

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
        event_type == KnowledgePageRecorded::TYPE
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        let registry = self.models.current();
        let vocabulary = self.vocabulary.clone();
        let event = event.clone();
        self.db
            .transaction(move |tx| {
                write_record_tx(tx, &registry, &vocabulary.current(), &event, Cause::Trigger)
                    .map(|_| ())
            })
            .await
    }
}

/// The provider fill of `store` in `model`: the one `with_provider_fills`
/// added for the running instance. A store an extension fills is not here —
/// that extension writes it, and the adapter writing it too would give it a
/// second writer.
fn provider_fill<'a>(
    registry: &'a crate::models::ModelRegistry,
    model: &str,
    store: &str,
) -> Option<&'a crate::models::FillSpec> {
    registry
        .fills
        .iter()
        .find(|f| f.provider && f.model == model && f.stores.iter().any(|s| s == store))
}

/// Write the record `event` states into its model, as its instance's
/// provider fill; `Ok(false)` when it is not this adapter's (no provider
/// fill: the instance isn't running, or bundled's model isn't declared).
fn write_record_tx(
    tx: &rusqlite::Connection,
    registry: &crate::models::ModelRegistry,
    vocabulary: &oxplow_domain::vocabulary::Vocabulary,
    event: &StoredEvent,
    why: Cause,
) -> Result<bool, DomainError> {
    let env = &event.envelope;
    let KnowledgePageRecordedV3 { page } = serde_json::from_value(env.payload.clone())
        .map_err(|e| invalid(format!("knowledge.page.recorded payload: {e}")))?;
    let Some((instance, id)) = oxplow_domain::uri::build::knowledge_of_uri(&page.uri) else {
        return Ok(false);
    };
    let model = KNOWLEDGE_RECORD;
    let Some(fill) = provider_fill(registry, model, &instance) else {
        return Ok(false);
    };
    let (upsert, delete) = if page.deleted {
        (Vec::new(), vec![id.clone()])
    } else {
        (vec![page_row(&id, &page)], Vec::new())
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
    let source = oxplow_domain::uri::build::provider_source(&instance);
    let writer = Writer {
        extension: &fill.extension,
        source: &source,
        cause: Some(env.id.clone()),
        anchors: env.anchors.clone(),
        why,
    };
    let applied = apply_tx(tx, registry, vocabulary, &writer, &[output], &[]).map_err(invalid)?;
    for problem in applied.problems {
        tracing::warn!(%instance, %problem, "a provider record wasn't written");
    }
    Ok(true)
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

/// State each provider fill's store once, when the fill appears.
///
/// The consumer drops a record that arrives with no provider fill (bundled
/// off, the model undeclared, the instance not yet in the registry), and
/// the provider sync never sends that record again (it equals the page's
/// last record). So a publication that carries a provider fill
/// not stated before writes, from the event log, the latest record of
/// each item of that store, with cause `reconcile`; a deleted item stays
/// out. `asset_state` keeps a `model_reconcile:<model>:<store>` marker for
/// each fill stated; the marker of a fill the publication no longer has is
/// dropped, so a fill that goes away and comes back is stated again. The
/// publication runs it after the capability rows are restated (a task's
/// stage is read from them).
pub fn reconcile_if_needed_tx(
    conn: &rusqlite::Connection,
    registry: &crate::models::ModelRegistry,
    vocabulary: &oxplow_domain::vocabulary::Vocabulary,
) -> Result<(), DomainError> {
    let storage = oxplow_db::map_sql_err;
    let wanted: Vec<(String, String, String)> = registry
        .fills
        .iter()
        .filter(|f| f.provider)
        .flat_map(|f| {
            f.stores
                .iter()
                .map(|s| (f.model.clone(), s.clone(), f.extension.clone()))
        })
        .collect();
    let assets: Vec<String> = wanted
        .iter()
        .map(|(m, s, _)| format!("model_reconcile:{m}:{s}"))
        .collect();
    let held: Vec<String> = conn
        .prepare("SELECT asset FROM asset_state WHERE asset LIKE 'model_reconcile:%'")
        .and_then(|mut st| st.query_map([], |r| r.get(0))?.collect())
        .map_err(storage)?;
    for gone in held.iter().filter(|a| !assets.contains(a)) {
        conn.execute("DELETE FROM asset_state WHERE asset = ?1", [gone])
            .map_err(storage)?;
    }
    let fresh: Vec<&(String, String, String)> = wanted
        .iter()
        .zip(&assets)
        .filter(|(_, asset)| !held.contains(asset))
        .map(|(w, _)| w)
        .collect();
    if fresh.is_empty() {
        return Ok(());
    }
    let started = std::time::Instant::now();
    {
        let (event_type, key) = (KnowledgePageRecorded::TYPE, "page");
        let ids: Vec<String> = conn
            .prepare(&format!(
                "SELECT id FROM event_log WHERE type = ?1 AND seq IN (
                   SELECT max(seq) FROM event_log WHERE type = ?1
                    GROUP BY json_extract(payload, '$.{key}.uri'))
                 ORDER BY seq"
            ))
            .and_then(|mut st| st.query_map([event_type], |r| r.get(0))?.collect())
            .map_err(storage)?;
        for id in ids {
            let Some(event) =
                oxplow_db::event_log_store::get_tx(conn, &oxplow_domain::EventId(id))?
            else {
                continue;
            };
            if event.payload_expired_at.is_some() {
                continue;
            }
            // Only a store stated now; one stated before follows its events.
            let store =
                serde_json::from_value::<KnowledgePageRecordedV3>(event.envelope.payload.clone())
                    .ok()
                    .and_then(|p| oxplow_domain::uri::build::knowledge_of_uri(&p.page.uri));
            let Some((instance, _)) = store else { continue };
            if !fresh
                .iter()
                .any(|(m, s, _)| m == KNOWLEDGE_RECORD && *s == instance)
            {
                continue;
            }
            // One record that won't write must not stop a publication.
            if let Err(error) =
                write_record_tx(conn, registry, vocabulary, &event, Cause::Reconcile)
            {
                tracing::warn!(%instance, %error, "a provider record wasn't reconciled");
            }
        }
    }
    for ((model, store, extension), asset) in wanted.iter().zip(&assets) {
        if !fresh.iter().any(|(m, s, _)| m == model && s == store) {
            continue;
        }
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
                extension
            ],
        )
        .map_err(storage)?;
    }
    Ok(())
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
