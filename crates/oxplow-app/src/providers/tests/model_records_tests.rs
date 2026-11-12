//! The provider read adapter over the fake provider: its records land in
//! bundled's record models under the instance's store.

use std::time::Duration;

use serde_json::Value;

use super::*;
use crate::providers::model_records::ModelRecords;

const TASK_STORE: &str = "fake";

/// The fixture with the fake tracker running; it declares one field of
/// its own, `points`.
#[cfg(unix)]
async fn running_tracker() -> ThreadFixture {
    let fx = services_with_thread().await;
    let project = fx.svc.layout.project_dir.clone();
    write_extension(&project, "");
    let manifest = project.join(".oxplow/extensions/tracker/extension.yaml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text.replacen(
            "    stages: [",
            "    fields: [{ name: points, title: Points, kind: number }]\n    stages: [",
            1,
        ),
    )
    .unwrap();
    let ext = extension(&project);
    deploy(&fx, &ext).await;
    make_active(&fx, "fake").await;
    crate::test_fixtures::activate(&fx.svc).await;
    enable_on(&fx.svc, &ext, &ext.providers[0], json!({ "team": "core" })).await;
    first_read(&fx).await;
    fx
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
async fn a_recorded_task_lands_in_the_record_model_and_a_deleted_one_goes() {
    let fx = running_tracker().await;
    let items = fx.svc.tasks_client();
    let item = items
        .create(
            &Actor::Human,
            crate::tasks::NewItem {
                title: "theirs".into(),
                body: "see it".into(),
                stage: Some("doing".into()),
                native: Some(json!({ "points": 3 })),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .expect("the list keeps it");
    items
        .link(
            &Actor::Human,
            &item,
            "oxplow:oxplow/file?path=src/lib.rs",
            "relates_to",
        )
        .await
        .unwrap();
    items.comment(&Actor::Human, &item, "first!").await.unwrap();
    settled(&fx).await;

    let task = rows(
        &fx,
        &format!(
            "SELECT id, title, body, state, stage, stage_title, fields FROM v_oxplow_task_record WHERE store = '{TASK_STORE}'"
        ),
    )
    .await;
    assert_eq!(task.len(), 1, "{task:?}");
    let t = &task[0];
    assert_eq!(t[1], json!("theirs"));
    assert_eq!(t[2], json!("see it"));
    assert_eq!(t[3], json!("in_progress"));
    assert_eq!(t[4], json!("doing"));
    assert_eq!(t[5], json!("Doing"));
    assert!(t[6].to_string().contains("points"), "{}", t[6]);
    let links = rows(
        &fx,
        &format!(
            "SELECT \"to\", type FROM v_oxplow_task_record_links WHERE store = '{TASK_STORE}'"
        ),
    )
    .await;
    assert_eq!(
        links,
        vec![vec![
            json!("oxplow:oxplow/file?path=src/lib.rs"),
            json!("relates_to")
        ]]
    );
    let comments = rows(
        &fx,
        &format!("SELECT body FROM v_oxplow_task_record_comments WHERE store = '{TASK_STORE}'"),
    )
    .await;
    assert_eq!(comments, vec![vec![json!("first!")]]);

    // A move restates the row; closing sets when.
    items
        .transition(&Actor::Human, &item, "shipped")
        .await
        .unwrap();
    settled(&fx).await;
    let moved = rows(
        &fx,
        &format!(
            "SELECT state, closed_at IS NOT NULL FROM v_oxplow_task_record WHERE store = '{TASK_STORE}'"
        ),
    )
    .await;
    assert_eq!(moved, vec![vec![json!("done"), json!(1)]]);

    // Its change event is logged.
    let changed = logged(&fx, "oxplow.task_record.changed").await;
    assert!(!changed.is_empty());

    items.delete(&Actor::Human, &item, true).await.unwrap();
    settled(&fx).await;
    let gone = rows(
        &fx,
        &format!("SELECT id FROM v_oxplow_task_record WHERE store = '{TASK_STORE}'"),
    )
    .await;
    assert!(gone.is_empty(), "{gone:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn with_no_model_declared_the_adapter_does_nothing_and_fails_nothing() {
    let fx = running_tracker().await;
    // Bundled off: no model is declared.
    fx.svc.models.swap(crate::models::ModelRegistry::default());
    fx.svc
        .tasks_client()
        .create(
            &Actor::Human,
            crate::tasks::NewItem {
                title: "x".into(),
                ..Default::default()
            },
        )
        .await
        .unwrap();
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
    let held = rows(&fx, "SELECT id FROM v_oxplow_task_record").await;
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
