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

/// A script's `set_fields` write is undone with its run: the field goes back
/// to what it was (a value, then none).
#[tokio::test(flavor = "multi_thread")]
async fn undoing_a_scripts_field_write_restores_the_field() {
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
    let fields = || async {
        let text: String = fx
            .svc
            .db
            .read(|tx| {
                tx.query_row("SELECT fields FROM v_shelf_book", [], |r| r.get(0))
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        serde_json::from_str::<serde_json::Value>(&text).unwrap()
    };
    let first = run("scorer.book.score", json!({ "uri": DUNE, "score": 9 }))
        .await
        .unwrap();
    let second = run("scorer.book.score", json!({ "uri": DUNE, "score": 4 }))
        .await
        .unwrap();
    assert_eq!(fields().await, json!({ "scorer": { "score": 4 } }));
    fx.svc
        .commands
        .undo(&Actor::Human, second.audit_id.unwrap(), true)
        .await
        .unwrap();
    assert_eq!(fields().await, json!({ "scorer": { "score": 9 } }));
    fx.svc
        .commands
        .undo(&Actor::Human, first.audit_id.unwrap(), true)
        .await
        .unwrap();
    assert_eq!(fields().await, json!({}));
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

/// `shelf` and its sqlite store `shelfdb` (a snapshot collector, and a
/// command `book.add` that inserts a row) on, and an agent mid-turn.
async fn shelfdb_with_an_agent_mid_turn() -> (crate::test_fixtures::ThreadFixture, Actor) {
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
    // An agent mid-turn.
    fx.svc
        .hook_ingest
        .ingest(crate::HookEnvelope {
            session_title: None,
            turn_run: None,
            reading: Default::default(),
            kind: oxplow_domain::HookKind::UserPromptSubmit,
            thread_id: Some(fx.thread),
            stream_id: None,
            agent_session_id: Some(fx.session),
            session_id: Some("native".into()),
            payload_json: "{}".into(),
            prompt: None,
            decision: None::<crate::ToolDecision>,
            tool: crate::test_fixtures::claude_tool(
                oxplow_domain::HookKind::UserPromptSubmit,
                &json!({}),
            ),
            subagent: None,
            harness_turn: None,
        })
        .await
        .unwrap();
    let agent = Actor::Agent {
        session_id: Some(fx.session),
        thread_id: Some(fx.thread),
        stream_id: None,
    };
    (fx, agent)
}

/// A store's command that writes its own file is observed at once, as
/// that command: the model shows the row when the command returns, and its
/// change event carries the command's anchors (the agent's thread).
#[tokio::test(flavor = "multi_thread")]
async fn a_stores_write_shows_in_the_model_when_its_command_returns() {
    let (fx, agent) = shelfdb_with_an_agent_mid_turn().await;
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
    assert!(
        changed[0].envelope.anchors.turn_id.is_some(),
        "and its open turn's: {:?}",
        changed[0].envelope.anchors
    );
}

/// A write to the store's file from outside (a person's sqlite3) that no
/// trigger has observed yet is nobody's, even when a command's write is
/// observed next: the command's turn takes only its own change.
#[tokio::test(flavor = "multi_thread")]
async fn an_outside_write_pending_when_a_command_writes_is_nobodys() {
    let (fx, agent) = shelfdb_with_an_agent_mid_turn().await;
    let add = |id: &'static str| {
        let (svc, agent) = (fx.svc.clone(), agent.clone());
        async move {
            svc.commands
                .run(
                    &agent,
                    "shelfdb.book.add",
                    json!({ "id": id, "title": id }),
                    true,
                )
                .await
                .unwrap();
        }
    };
    add("dune").await;
    let file = crate::extension::store_path(fx._dir.path(), "shelfdb");
    rusqlite::Connection::open(&file)
        .unwrap()
        .execute("INSERT INTO book (id, title) VALUES ('emma', 'Emma')", [])
        .unwrap();
    add("carrie").await;
    let threads: Vec<(String, Option<oxplow_domain::ThreadId>)> = fx
        .svc
        .event_log_store
        .read_after(0, 1000)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| e.envelope.event_type == "shelf.book.changed")
        .map(|e| {
            (
                e.envelope.payload["uri"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
                e.envelope.anchors.thread_id,
            )
        })
        .collect();
    assert_eq!(threads.len(), 3, "{threads:?}");
    for (uri, thread) in &threads {
        let want = (!uri.contains("id=emma")).then_some(fx.thread);
        assert_eq!(*thread, want, "{uri}");
    }
}

/// A task store that declares how its ids look (`fills.id_pattern`) makes a
/// loose id in text its item, beside the active tracker's.
#[tokio::test(flavor = "multi_thread")]
async fn an_id_pattern_on_a_fill_recognizes_its_loose_task_ids() {
    let fx = services_with_thread().await;
    let root = fx._dir.path();
    write(
        root,
        ".oxplow/extensions/zz/extension.yaml",
        "manifest: 2\nname: zz\nintent:\n  purpose: p\nfills:\n  - model: oxplow/task_record\n    stores: [zz]\n    id_pattern: \"zz[0-9]+\"\n",
    );
    let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert!(problems.is_empty(), "{problems:?}");
    let resources = fx.svc.vocabulary.current().resources.clone();
    assert_eq!(
        resources.task_uri_of("zz4").as_deref(),
        Some("oxplow:oxplow/task?id=zz4&provider=zz")
    );
    assert_eq!(resources.task_uri_of("zzz"), None);
}

/// A script's inverse can't name `oxplow.model.set_fields`: that command
/// records its own undo, and naming it with `extension` would write another
/// extension's fields when the person undoes.
#[tokio::test(flavor = "multi_thread")]
async fn a_scripts_inverse_cant_write_fields_as_another_extension() {
    let fx = services_with_thread().await;
    let root = fx._dir.path();
    write(root, ".oxplow/extensions/shelf/extension.yaml", OWNER);
    write(
        root,
        ".oxplow/extensions/shelfdb/extension.yaml",
        &DB_STORE.replace(
            "permissions: ['commands:oxplow.extension_store.write']",
            "permissions: ['commands:oxplow.extension_store.write', 'commands:oxplow.model.set_fields']",
        ),
    );
    write(
        root,
        ".oxplow/extensions/shelfdb/storage/migrations/V1__book.sql",
        "CREATE TABLE book (id TEXT PRIMARY KEY, title TEXT NOT NULL);",
    );
    write(
        root,
        ".oxplow/extensions/shelfdb/books.star",
        "def transform(x):\n    return {\"models\": []}\n",
    );
    write(
        root,
        ".oxplow/extensions/shelfdb/handlers/add.star",
        "def transform(x):\n    return {\"commands\": [{\"name\": \"oxplow.extension_store.write\", \"input\": {\"batch\": [{\"sql\": \"INSERT INTO book (id, title) VALUES ('a', 'A')\", \"params\": []}]}}], \"result\": {}, \"inverse\": {\"commands\": [{\"name\": \"oxplow.model.set_fields\", \"input\": {\"model\": \"shelf/book\", \"uri\": \"oxplow:shelf/book?id=a&store=shelfdb\", \"values\": {\"score\": None}, \"extension\": \"shelf\"}}]}}\n",
    );
    let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert!(problems.is_empty(), "{problems:?}");
    let err = fx
        .svc
        .commands
        .run(&Actor::Human, "shelfdb.book.add", json!({}), true)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("oxplow.model.set_fields"), "{err}");
    assert!(err.contains("records its own undo"), "{err}");
}

/// A store's declared stages are read as `v_model_stage`, in order, with
/// their buckets: what shared screens draw a store's workflow from.
#[tokio::test(flavor = "multi_thread")]
async fn a_stores_stages_are_read_as_model_stage() {
    let fx = services_with_thread().await;
    let root = fx._dir.path();
    write(
        root,
        ".oxplow/extensions/zz/extension.yaml",
        "manifest: 2\nname: zz\nintent:\n  purpose: p\nfills:\n  - model: oxplow/task_record\n    stores: [zz]\n    stages:\n      - { id: todo, title: To do, bucket: todo }\n      - { id: doing, title: Doing, bucket: in_progress }\n      - { id: done, title: Done, bucket: done }\n",
    );
    let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert!(problems.is_empty(), "{problems:?}");
    let out = fx
        .svc
        .sql
        .query_sql(
            "SELECT id || ':' || bucket FROM v_model_stage WHERE model = 'oxplow/task_record' AND store = 'zz' ORDER BY position",
            vec![],
            None,
        )
        .await
        .unwrap();
    let got: Vec<String> = out
        .rows
        .iter()
        .map(|r| match &r[0] {
            oxplow_db::SqlCell::Text(t) => t.clone(),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(got, vec!["todo:todo", "doing:in_progress", "done:done"]);
}

/// A model's children are its items' related rows when one is read
/// (`oxplow_read`): each child set by its name, the item's own only.
#[tokio::test(flavor = "multi_thread")]
async fn a_models_children_are_read_beside_its_item() {
    let fx = services_with_thread().await;
    let root = fx._dir.path();
    let owner = OWNER
        .replace(
            "    params: { store: { required: true }, id: { required: true } }\n",
            "    params: { store: { required: true }, id: { required: true } }\n    resolve: books\ndatasets:\n  - name: books\n    version: 1\n    description: Books.\n    fields:\n      - { name: uri, type: TEXT, doc: u }\n      - { name: title, type: TEXT, doc: t }\n",
        )
        .replace(
            "    columns: { id: text, title: text }\n",
            "    columns: { id: text, title: text }\n    children:\n      notes:\n        key: [n]\n        columns: { n: text, text: text }\n",
        );
    write(root, ".oxplow/extensions/shelf/extension.yaml", &owner);
    write(
        root,
        ".oxplow/extensions/shelf/datasets/books.sql",
        "SELECT uri, title FROM ref('book')",
    );
    write(root, ".oxplow/extensions/shelves/extension.yaml", STORE);
    write(
        root,
        ".oxplow/extensions/shelves/books.star",
        "def transform(x):\n    return {\"models\": [{\"model\": \"shelf/book\", \"store\": \"shelves\", \"snapshot\": [{\"id\": \"dune\", \"title\": \"Dune\", \"notes\": [{\"n\": \"1\", \"text\": \"Spice\"}]}, {\"id\": \"emma\", \"title\": \"Emma\", \"notes\": [{\"n\": \"1\", \"text\": \"Bath\"}]}]}]}\n",
    );
    let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert!(problems.is_empty(), "{problems:?}");
    fx.svc
        .commands
        .run(
            &Actor::Human,
            crate::collector_runner::SYNC,
            json!({ "collector": "shelves/books" }),
            false,
        )
        .await
        .unwrap();
    let read = crate::ref_read::read(
        &fx.svc,
        "oxplow:shelf/book?id=dune&store=shelves",
        &crate::ref_read::Reader::default(),
        None,
    )
    .await
    .unwrap();
    let notes = &read.related["notes"];
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0]["text"], "Spice");
}

/// `needs: [model:<ns>/<model>]` is met while the model has a live store:
/// what turns task guidance on and off once tasks are a model's.
#[tokio::test(flavor = "multi_thread")]
async fn a_model_need_is_met_while_the_model_has_a_live_store() {
    let fx = services_with_thread().await;
    let need = vec!["model:oxplow/task_record".to_string()];
    let active = || {
        fx.svc
            .capabilities
            .snapshot(&crate::config_service::read_config(&fx.svc.config))
    };
    crate::test_fixtures::activate(&fx.svc).await;
    assert_eq!(active().unmet(&need), need, "no store fills it yet");
    write(
        fx._dir.path(),
        ".oxplow/extensions/zz/extension.yaml",
        "manifest: 2\nname: zz\nintent:\n  purpose: p\nfills:\n  - model: oxplow/task_record\n    stores: [zz]\n",
    );
    let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert!(problems.is_empty(), "{problems:?}");
    assert!(active().unmet(&need).is_empty(), "zz fills it");
}
