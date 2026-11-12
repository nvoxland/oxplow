//! End to end: an owner declares a shared model, a store's collector fills
//! it, and another extension's command composes `oxplow.model.set_fields`
//! to add its own field to the store's item.

use oxplow_domain::{Actor, CommandError};
use serde_json::json;

use crate::extension::extension_private_state_tests::write;
use crate::test_fixtures::services_with_thread;

const OWNER: &str = "manifest: 2\nname: shelf\nintent:\n  purpose: p\nresources:\n  - name: book\n    description: A book.\n    label: Book\n    params: { store: { required: true }, id: { required: true } }\nmodels:\n  - name: book\n    resource: book\n    open: true\n    key: id\n    change_event: shelf.book.changed\n    search: { title: title, body: title }\n    columns: { id: text, title: text }\n";

const STORE: &str = "manifest: 2\nname: shelves\nintent:\n  purpose: p\nrequires: [shelf]\nfills:\n  - model: shelf/book\n    stores: [shelves]\ncollectors:\n  - id: books\n    runtime: starlark\n    entry: books.star\n";

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
        "def transform(x):\n    return {\"models\": [{\"model\": \"shelf/book\", \"store\": \"shelves\", \"snapshot\": [{\"id\": \"dune\", \"title\": \"Dune\"}, {\"id\": \"odd\", \"colour\": \"red\"}]}]}\n",
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
    let synced = run(
        crate::collector_runner::SYNC,
        json!({ "collector": "shelves/books" }),
    )
    .await
    .unwrap();
    // The bad row is skipped and named; the good one lands.
    assert!(
        synced.result.to_string().contains("colour"),
        "{}",
        synced.result
    );
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

/// A collector that only fills a shared model needs no entities; one with
/// nowhere to write (no entities, no facts, no fill) is refused at its line.
#[tokio::test(flavor = "multi_thread")]
async fn a_collector_with_nowhere_to_write_is_refused() {
    let fx = services_with_thread().await;
    let root = fx._dir.path();
    write(
        root,
        ".oxplow/extensions/idle/extension.yaml",
        "manifest: 2\nname: idle\nintent:\n  purpose: p\ncollectors:\n  - id: nothing\n    runtime: starlark\n    entry: nothing.star\n",
    );
    write(
        root,
        ".oxplow/extensions/idle/nothing.star",
        "def transform(x):\n    return {}\n",
    );
    let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
    let listed = fx.svc.listed_extensions(root).await;
    let idle = listed.iter().find(|e| e.name == "idle").unwrap();
    let text = format!("{:?} {problems:?}", idle.errors);
    assert!(
        text.contains("collector.no_output") && text.contains("fills no shared model"),
        "{text}"
    );
}

/// Two extensions can't write one store id into one model: the later one
/// is refused, naming the store and who writes it.
#[tokio::test(flavor = "multi_thread")]
async fn a_store_id_two_extensions_claim_is_refused() {
    let fx = services_with_thread().await;
    let root = fx._dir.path();
    write(root, ".oxplow/extensions/shelf/extension.yaml", OWNER);
    write(root, ".oxplow/extensions/shelves/extension.yaml", STORE);
    write(
        root,
        ".oxplow/extensions/shelves/books.star",
        "def transform(x):\n    return {}\n",
    );
    write(
        root,
        ".oxplow/extensions/squatter/extension.yaml",
        "manifest: 2\nname: squatter\nintent:\n  purpose: p\nrequires: [shelf]\nfills:\n  - model: shelf/book\n    stores: [shelves]\n",
    );
    let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
    let text = format!("{problems:?}");
    assert!(
        text.contains("model.store_taken") && text.contains("shelves"),
        "{text}"
    );
}

const DB_STORE: &str = "manifest: 2\nname: shelfdb\nintent:\n  purpose: p\nrequires: [shelf]\npermissions: ['commands:oxplow.extension_store.write']\nstorage: { sqlite: true }\nfills:\n  - model: shelf/book\n    stores: [shelfdb]\ncollectors:\n  - id: books\n    runtime: starlark\n    entry: books.star\n    needs: [store.sql]\n    on: [{ sqlite: own }]\ncommands:\n  - name: book.add\n    summary: Add a book.\n    input_schema: { type: object }\n    entry: handlers/add.star\n";

/// A store's command that writes its own file is observed at once, as
/// that command: the model shows the row when the command returns, and its
/// change event carries the command's anchors (the agent's thread).
#[tokio::test(flavor = "multi_thread")]
async fn a_stores_write_shows_in_the_model_when_its_command_returns() {
    let fx = services_with_thread().await;
    let root = fx._dir.path();
    write(root, ".oxplow/extensions/shelf/extension.yaml", OWNER);
    write(root, ".oxplow/extensions/shelfdb/extension.yaml", DB_STORE);
    write(
        root,
        ".oxplow/extensions/shelfdb/storage/migrations/V1__book.sql",
        "CREATE TABLE book (id TEXT PRIMARY KEY, title TEXT NOT NULL);",
    );
    write(
        root,
        ".oxplow/extensions/shelfdb/books.star",
        "def transform(x):\n    rows = scope(\"store.sql\", {\"sql\": \"SELECT id, title FROM book\"})[\"rows\"]\n    return {\"models\": [{\"model\": \"shelf/book\", \"store\": \"shelfdb\", \"snapshot\": rows}]}\n",
    );
    write(
        root,
        ".oxplow/extensions/shelfdb/handlers/add.star",
        "def transform(x):\n    return {\"commands\": [{\"name\": \"oxplow.extension_store.write\", \"input\": {\"batch\": [{\"sql\": \"INSERT INTO book (id, title) VALUES (?1, ?2)\", \"params\": [x[\"input\"][\"id\"], x[\"input\"][\"title\"]]}]}}], \"result\": {}}\n",
    );
    let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert!(problems.is_empty(), "{problems:?}");
    let agent = Actor::Agent {
        session_id: None,
        thread_id: Some(fx.thread),
        stream_id: None,
    };
    fx.svc
        .commands
        .run(
            &agent,
            "shelfdb.book.add",
            json!({ "id": "dune", "title": "Dune" }),
            true,
        )
        .await
        .unwrap();
    let title: String = fx
        .svc
        .db
        .read(|tx| {
            tx.query_row(
                "SELECT title FROM v_shelf_book WHERE store = 'shelfdb'",
                [],
                |r| r.get(0),
            )
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap();
    assert_eq!(title, "Dune", "the row is there when the command returns");
    let changed: Vec<_> = fx
        .svc
        .event_log_store
        .read_after(0, 1000)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.envelope.event_type == "shelf.book.changed")
        .collect();
    assert_eq!(changed.len(), 1);
    assert_eq!(
        changed[0].envelope.anchors.thread_id,
        Some(fx.thread),
        "the change is the command's thread's"
    );
}
