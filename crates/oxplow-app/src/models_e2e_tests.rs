//! End to end: an owner declares a shared model, a store's collector fills
//! it, and another extension's command composes `oxplow.model.set_fields`
//! to add its own field to the store's item.

use oxplow_domain::{Actor, CommandError};
use serde_json::json;

use crate::extension::extension_private_state_tests::write;
use crate::test_fixtures::services_with_thread;

const OWNER: &str = "manifest: 2\nname: shelf\nintent:\n  purpose: p\nresources:\n  - name: book\n    description: A book.\n    label: Book\n    params: { store: { required: true }, id: { required: true } }\nmodels:\n  - name: book\n    resource: book\n    open: true\n    key: id\n    change_event: shelf.book.changed\n    search: { title: title, body: title }\n    columns: { id: text, title: text }\n";

const STORE: &str = "manifest: 2\nname: shelves\nintent:\n  purpose: p\nrequires: [shelf]\nfills:\n  - model: shelf/book\n    stores: [shelves]\ncollectors:\n  - id: books\n    runtime: starlark\n    entry: books.star\n    entities:\n      - { name: seen, key: id, columns: { id: text } }\n";

const SCORER: &str = "manifest: 2\nname: scorer\nintent:\n  purpose: p\nrequires: [shelf]\npermissions: ['commands:oxplow.model.set_fields']\nfields:\n  - model: shelf/book\n    fields: [{ name: score, title: Score, kind: number }]\ncommands:\n  - name: book.score\n    summary: Score a book.\n    input_schema: { type: object }\n    entry: handlers/score.star\n";

const DUNE: &str = "oxplow:shelf/book?id=dune&store=shelves";

/// A script command adds its own field to a store's item by composing
/// `oxplow.model.set_fields`; nobody can call it directly.
#[tokio::test(flavor = "multi_thread")]
async fn a_script_sets_its_own_field_on_a_stores_item() {
    let fx = services_with_thread().await;
    let root = fx._dir.path();
    write(root, ".oxplow/extensions/shelf/extension.yaml", OWNER);
    write(root, ".oxplow/extensions/shelves/extension.yaml", STORE);
    write(
        root,
        ".oxplow/extensions/shelves/books.star",
        "def transform(x):\n    return {\"models\": [{\"model\": \"shelf/book\", \"store\": \"shelves\", \"snapshot\": [{\"id\": \"dune\", \"title\": \"Dune\"}]}]}\n",
    );
    write(root, ".oxplow/extensions/scorer/extension.yaml", SCORER);
    write(
        root,
        ".oxplow/extensions/scorer/handlers/score.star",
        "def transform(x):\n    return {\"commands\": [{\"name\": \"oxplow.model.set_fields\", \"input\": {\"model\": \"shelf/book\", \"uri\": x[\"input\"][\"uri\"], \"values\": {\"score\": x[\"input\"][\"score\"]}}}], \"result\": {}}\n",
    );
    let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert!(problems.is_empty(), "{problems:?}");
    let run = |name: &'static str, input: serde_json::Value| {
        fx.svc.commands.run(&Actor::Human, name, input, false)
    };
    run(
        crate::collector_runner::SYNC,
        json!({ "collector": "shelves/books" }),
    )
    .await
    .unwrap();
    run("scorer.book.score", json!({ "uri": DUNE, "score": 9 }))
        .await
        .unwrap();
    let fields: String = fx
        .svc
        .db
        .read(|tx| {
            tx.query_row("SELECT fields FROM v_shelf_book", [], |r| r.get(0))
                .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&fields).unwrap(),
        json!({ "scorer": { "score": 9 } })
    );
    // The model's resource is searchable through its generated view, and
    // the index finds the store's item (the change loop, as boot runs it,
    // over assets that settle fast).
    let assets =
        crate::assets::Assets::new(fx.svc.db.clone(), std::time::Duration::from_millis(50));
    crate::datasets_changed::spawn(
        fx.svc.db.clone(),
        std::sync::Arc::new(crate::datasets_changed::DatasetWatermarks::default()),
        fx.svc.events.clone(),
        assets.clone(),
        fx.svc.event_pump.clone(),
        crate::pages::open_runs::PageRuns::default(),
    );
    assets.sync_search_kinds().await.unwrap();
    let mut hits = Vec::new();
    for _ in 0..100 {
        hits = fx
            .svc
            .search_store
            .search("Dune", None, &["shelf/book".to_string()], 10)
            .await
            .unwrap()
            .into_iter()
            .map(|h| h.uri)
            .collect();
        if !hits.is_empty() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(hits, vec![DUNE.to_string()]);
    let direct = run(
        "oxplow.model.set_fields",
        json!({ "model": "shelf/book", "uri": DUNE, "values": { "score": 1 } }),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&direct, CommandError::Denied { reason } if reason.contains("composed")),
        "{direct:?}"
    );
}
