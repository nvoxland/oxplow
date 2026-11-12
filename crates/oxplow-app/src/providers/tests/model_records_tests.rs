//! The provider read adapter over the fake provider: its records land in
//! bundled's record models under the instance's store.

use std::time::Duration;

use serde_json::Value;

use super::*;
use crate::providers::model_records::ModelRecords;

/// The store the fake provider's pages are recorded under: its instance id.
const STORE: &str = "fake";

/// The fixture with the fake knowledge provider running.
#[cfg(unix)]
async fn running_knowledge() -> ThreadFixture {
    enabled_knowledge().await.0
}

async fn settled(fx: &ThreadFixture) {
    assert!(
        fx.svc
            .event_pump
            .settle(&[ModelRecords::NAME], Duration::from_secs(10))
            .await,
        "the adapter caught up"
    );
}

async fn rows(fx: &ThreadFixture, sql: &str) -> Vec<Vec<Value>> {
    let sql = sql.to_string();
    fx.svc
        .db
        .read(move |c| {
            let mut st = c
                .prepare(&sql)
                .map_err(|e| oxplow_domain::DomainError::Storage(format!("{sql}: {e}")))?;
            let n = st.column_count();
            let out = st
                .query_map([], |r| {
                    (0..n)
                        .map(|i| {
                            Ok(match r.get_ref(i)? {
                                rusqlite::types::ValueRef::Null => Value::Null,
                                rusqlite::types::ValueRef::Integer(v) => json!(v),
                                rusqlite::types::ValueRef::Real(v) => json!(v),
                                rusqlite::types::ValueRef::Text(t) => {
                                    json!(String::from_utf8_lossy(t))
                                }
                                rusqlite::types::ValueRef::Blob(_) => Value::Null,
                            })
                        })
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .and_then(|m| m.collect::<rusqlite::Result<Vec<_>>>())
                .map_err(|e| oxplow_domain::DomainError::Storage(e.to_string()))?;
            Ok(out)
        })
        .await
        .unwrap()
}

#[cfg(unix)]
#[tokio::test]
async fn with_no_model_declared_the_adapter_does_nothing_and_fails_nothing() {
    let fx = running_knowledge().await;
    // Bundled off: no model is declared.
    fx.svc.models.swap(crate::models::ModelRegistry::default());
    page_on(&fx, INSTANCE, "x").await.unwrap();
    sync_pages(&fx).await.unwrap();
    settled(&fx).await;
    let dead = rows(
        &fx,
        &format!(
            "SELECT count(*) FROM event_dead_letter WHERE consumer = '{}'",
            ModelRecords::NAME
        ),
    )
    .await;
    assert_eq!(dead, vec![vec![json!(0)]]);
    let held = rows(
        &fx,
        &format!("SELECT id FROM v_oxplow_knowledge_record WHERE store = '{STORE}'"),
    )
    .await;
    assert!(held.is_empty(), "{held:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn a_recorded_page_lands_in_the_knowledge_record_model_with_its_refs_as_edges() {
    let fx = services_with_thread().await;
    let project = fx.svc.layout.project_dir.clone();
    let state = project.join(".oxplow/fake-knowledge.json");
    std::fs::create_dir_all(state.parent().unwrap()).unwrap();
    std::fs::write(
        &state,
        serde_json::to_string(&json!({
            "items": {}, "next": 1, "rev": 1, "answered": {},
            "pages": { "outside": { "rev": 1, "record": {
                "uri": "oxplow:oxplow/knowledge?id=outside&store=shelf", "title": "Outside",
                "body": "# Outside\n", "uris": ["oxplow:oxplow/file?path=src/a.rs"],
                "updated_at": "2026-10-09T00:00:01Z"
            } } }
        }))
        .unwrap(),
    )
    .unwrap();
    write_knowledge_extension(&project, &state);
    let ext = crate::extension::extensions::load_extensions(&project)
        .into_iter()
        .find(|e| e.name == "shelf")
        .expect("the knowledge extension loads");
    deploy(&fx, &ext).await;
    enable_on(&fx.svc, &ext, &ext.providers[0], json!({ "team": "core" })).await;
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    fx.svc
        .commands
        .run(
            &Actor::Human,
            sync::SYNC,
            json!({ "instance": "shelf/shelf" }),
            false,
        )
        .await
        .unwrap();
    settled(&fx).await;
    let page = rows(
        &fx,
        "SELECT id, title FROM v_oxplow_knowledge_record WHERE store = 'shelf'",
    )
    .await;
    assert_eq!(page, vec![vec![json!("outside"), json!("Outside")]]);
    let edges = rows(
        &fx,
        "SELECT target_uri FROM page_ref WHERE source_uri = 'oxplow:oxplow/knowledge?id=outside&store=shelf'",
    )
    .await;
    assert!(
        edges
            .iter()
            .any(|e| e[0] == json!("oxplow:oxplow/file?path=src/a.rs")),
        "{edges:?}"
    );
}

/// A store an extension fills has its own writer: the adapter writes only
/// through the fills it added for provider instances.
#[cfg(unix)]
#[tokio::test]
async fn a_record_for_a_store_an_extension_fills_writes_nothing() {
    let fx = running_knowledge().await;
    let mut registry = fx.svc.models.current().as_ref().clone();
    for fill in registry.fills.iter_mut().filter(|f| f.provider) {
        fill.provider = false;
    }
    fx.svc.models.swap(registry);
    page_on(&fx, INSTANCE, "theirs").await.unwrap();
    sync_pages(&fx).await.unwrap();
    settled(&fx).await;
    let held = rows(
        &fx,
        &format!("SELECT id FROM v_oxplow_knowledge_record WHERE store = '{STORE}'"),
    )
    .await;
    assert!(held.is_empty(), "{held:?}");
}

/// A record that came while no fill stood (bundled off) is stated when the
/// fill is back: the publication states the store once.
#[cfg(unix)]
#[tokio::test]
async fn records_made_while_there_was_no_fill_appear_when_the_fill_returns() {
    let fx = running_knowledge().await;
    let full = fx.svc.models.current().as_ref().clone();
    // A publication with bundled off: no fill, so its markers go.
    reconcile(&fx, &crate::models::ModelRegistry::default()).await;
    fx.svc.models.swap(crate::models::ModelRegistry::default());
    page_on(&fx, INSTANCE, "kept").await.unwrap();
    let gone = page_on(&fx, INSTANCE, "gone").await.unwrap();
    invoke_on(&fx, INSTANCE, "delete_page", json!({ "id": gone }))
        .await
        .unwrap();
    sync_pages(&fx).await.unwrap();
    settled(&fx).await;
    // Bundled back on: the next publication states what the log holds.
    fx.svc.models.swap(full.clone());
    assert!(rows(
        &fx,
        &format!("SELECT id FROM v_oxplow_knowledge_record WHERE store = '{STORE}'")
    )
    .await
    .is_empty());
    reconcile(&fx, &full).await;
    let pages = rows(
        &fx,
        &format!("SELECT title FROM v_oxplow_knowledge_record WHERE store = '{STORE}'"),
    )
    .await;
    assert_eq!(pages, vec![vec![json!("kept")]]);
    let changes = logged(&fx, "oxplow.knowledge_record.changed").await;
    let first = changes.len();
    assert!(
        changes.iter().any(|c| c["cause"] == json!("reconcile")),
        "{changes:?}"
    );
    // A second publication states nothing more.
    reconcile(&fx, &full).await;
    assert_eq!(
        logged(&fx, "oxplow.knowledge_record.changed").await.len(),
        first
    );
    let marker = rows(
        &fx,
        "SELECT asset FROM asset_state WHERE asset LIKE 'model_reconcile:%'",
    )
    .await;
    assert_eq!(marker.len(), 1, "{marker:?}");
}

/// What a publication runs after its capability rows stand.
#[cfg(unix)]
async fn reconcile(fx: &ThreadFixture, registry: &crate::models::ModelRegistry) {
    let registry = registry.clone();
    let vocabulary = fx.svc.vocabulary.current();
    fx.svc
        .db
        .transaction(move |tx| {
            crate::providers::model_records::reconcile_if_needed_tx(tx, &registry, &vocabulary)
        })
        .await
        .unwrap();
}
