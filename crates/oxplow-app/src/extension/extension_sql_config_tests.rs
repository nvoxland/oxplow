//! An extension's SQL reads its own configuration: a dataset, a page and an
//! advisory name a key with `config('/pointer')`, the value comes from the
//! extension's effective settings, and a change of settings applies without a
//! restart.

use std::time::Duration;

use oxplow_domain::Actor;
use serde_json::{json, Value};

use crate::test_fixtures::{services_with_thread, ThreadFixture};

/// Three edits of a file in one turn of a request, nothing else.
async fn three_edits(fx: &ThreadFixture) {
    let thread = fx.thread.value();
    let turn: i64 = fx
        .svc
        .db
        .transaction(move |c| {
            c.execute(
                "INSERT INTO request (thread_id) VALUES (?1)",
                rusqlite::params![thread],
            )
            .map_err(oxplow_db::map_sql_err)?;
            let request = c.last_insert_rowid();
            c.execute(
                "INSERT INTO agent_turn (thread_id, prompt, request_id, started_at)
                 VALUES (?1, 'p', ?2, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))",
                rusqlite::params![thread, request],
            )
            .map_err(oxplow_db::map_sql_err)?;
            Ok(c.last_insert_rowid())
        })
        .await
        .unwrap();
    for _ in 0..3 {
        fx.svc
            .tool_call_store
            .record(oxplow_db::NewToolCall {
                thread_id: thread,
                turn_id: Some(turn),
                tool: "edit".into(),
                kind: "edit".into(),
                path: Some("src/warm.rs".into()),
                ok: Some(true),
                ..Default::default()
            })
            .await
            .unwrap();
    }
}

async fn struggles(fx: &ThreadFixture) -> Value {
    serde_json::to_value(
        fx.svc
            .sql
            .query_sql("SELECT subject, count FROM v_oxplow_struggle", vec![], None)
            .await
            .unwrap()
            .rows,
    )
    .unwrap()
}

/// A person sets the struggle threshold: the project layer, through the
/// extension's own command.
async fn set_threshold(fx: &ThreadFixture, edits: i64) {
    fx.svc
        .commands
        .run(
            &Actor::Human,
            "oxplow.config.set",
            json!({"key":"oxplow.struggle.repeatedEdits", "value":edits, "level":"project"}),
            false,
        )
        .await
        .unwrap();
}

/// A project override changes what the dataset says: three edits are not
/// struggle at the default of five, and are once the project sets three.
#[tokio::test(flavor = "multi_thread")]
async fn a_project_setting_changes_a_bundled_datasets_result() {
    let fx = crate::test_fixtures::services_with_thread().await;
    three_edits(&fx).await;
    assert_eq!(struggles(&fx).await, json!([]), "the default is five");

    set_threshold(&fx, 3).await;
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assert_eq!(struggles(&fx).await, json!([["src/warm.rs", 3]]));
}

/// With the owner loop running, a settings write publishes the datasets that
/// read it by itself: no restart and no explicit activation.
#[tokio::test(flavor = "multi_thread")]
async fn a_settings_change_republishes_the_datasets_that_read_it() {
    let fx = crate::test_fixtures::services_with_thread().await;
    three_edits(&fx).await;
    fx.svc.extension_activation.spawn(&fx.svc);
    set_threshold(&fx, 3).await;
    // What the event pump does when the write's `command.executed` arrives.
    fx.svc
        .extension_activation
        .request_config_refresh(
            &fx.svc.db,
            &fx.svc.layout.project_dir,
            &fx.svc.global_config_dir,
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if struggles(&fx).await != json!([]) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("the dataset republished on its own");
    assert_eq!(struggles(&fx).await, json!([["src/warm.rs", 3]]));
}

/// SQL an extension runs (a page's, an advisory's) reads the setting each
/// time: lowering the threshold changes the query it gets.
#[tokio::test(flavor = "multi_thread")]
async fn a_query_reads_the_current_setting() {
    let fx = services_with_thread().await;
    let bundled = fx
        .svc
        .extension_catalog
        .unpinned(&fx.svc.layout.project_dir, "a test reads declarations")
        .iter()
        .find(|e| e.name == "oxplow-bundled")
        .cloned()
        .unwrap();
    let query = "SELECT 1 WHERE 9 >= config('/struggle/repeatedEdits')";
    let before = fx
        .svc
        .sql
        .with_extension_config(&bundled, query)
        .await
        .unwrap();
    assert!(before.contains(">= 5"), "{before}");

    set_threshold(&fx, 1).await;
    let after = fx
        .svc
        .sql
        .with_extension_config(&bundled, query)
        .await
        .unwrap();
    assert!(after.ends_with(">= 1"), "{after}");
}

fn write(root: &std::path::Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

const CONFIGURED: &str = "manifest: 2\nname: mine\nintent:\n  purpose: p\n  examples: [{ name: a }]\nconfiguration:\n  version: 1\n  schema:\n    type: object\n    properties:\n      limit: { type: integer }\n      nested: { type: object }\n  defaults: { limit: 7, nested: {} }\ndatasets:\n  - name: limited\n    version: 1\n    description: Limited.\n    fields:\n      - { name: n, type: \"\", doc: The limit. }\n";

/// Another extension's settings are not readable: `config()` is the owner's,
/// so a key the owner's schema doesn't declare fails the load — naming the
/// key — and the other extension's value never appears.
#[tokio::test(flavor = "multi_thread")]
async fn an_extension_reads_only_its_own_settings() {
    let fx = services_with_thread().await;
    let root = fx.svc.layout.project_dir.clone();
    write(&root, ".oxplow/extensions/mine/extension.yaml", CONFIGURED);
    // `struggle` is oxplow-bundled's key, not `mine`'s.
    write(
        &root,
        ".oxplow/extensions/mine/datasets/limited.sql",
        "SELECT config('/struggle/repeatedEdits') AS n",
    );
    let load_errors = held_errors(&fx, "mine").await;
    let errors = load_errors;
    assert!(
        errors.contains("config('/struggle/repeatedEdits') names no key"),
        "{errors}"
    );

    // And one that declares no configuration can't read any.
    write(
        &root,
        ".oxplow/extensions/mine/extension.yaml",
        &CONFIGURED.replace("configuration:", "unused_marker_removed:").replace(
            "unused_marker_removed:\n  version: 1\n  schema:\n    type: object\n    properties:\n      limit: { type: integer }\n      nested: { type: object }\n  defaults: { limit: 7, nested: {} }\n",
            "",
        ),
    );
    write(
        &root,
        ".oxplow/extensions/mine/datasets/limited.sql",
        "SELECT config('/limit') AS n",
    );
    let load_errors = held_errors(&fx, "mine").await;
    assert!(
        load_errors.contains("which it doesn't declare"),
        "{load_errors}"
    );
}

async fn held_errors(fx: &ThreadFixture, name: &str) -> String {
    crate::test_fixtures::load_errors(&fx.svc, name).await
}

/// An extension's own key compiles into its dataset, and the reads are checked
/// at load: an object value has no SQL literal.
#[tokio::test(flavor = "multi_thread")]
async fn an_own_key_reads_and_a_key_that_is_not_a_scalar_fails_the_load() {
    let fx = services_with_thread().await;
    let root = fx.svc.layout.project_dir.clone();
    write(&root, ".oxplow/extensions/mine/extension.yaml", CONFIGURED);
    write(
        &root,
        ".oxplow/extensions/mine/datasets/limited.sql",
        "SELECT config('/limit') AS n",
    );
    let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
    let n = serde_json::to_value(
        fx.svc
            .sql
            .query_sql("SELECT n FROM v_mine_limited", vec![], None)
            .await
            .unwrap_or_else(|e| panic!("{e}: {problems:?}"))
            .rows,
    )
    .unwrap();
    assert_eq!(n, json!([[7]]));

    write(
        &root,
        ".oxplow/extensions/mine/datasets/limited.sql",
        "SELECT config('/nested') AS n",
    );
    let load_errors = held_errors(&fx, "mine").await;
    assert!(
        load_errors.contains("scalar `type`") || load_errors.contains("is a `object`"),
        "{load_errors}"
    );
}

/// A materialized dataset that reads a setting is refilled with the new value
/// when the setting changes, not left holding the old one until its timer.
#[tokio::test(flavor = "multi_thread")]
async fn a_materialized_dataset_is_refilled_when_its_setting_changes() {
    let fx = services_with_thread().await;
    let root = fx.svc.layout.project_dir.clone();
    write(
        &root,
        ".oxplow/extensions/mine/extension.yaml",
        &CONFIGURED.replace(
            "    version: 1\n    description: Limited.",
            "    version: 1\n    materialize: { every: 1h }\n    description: Limited.",
        ),
    );
    write(
        &root,
        ".oxplow/extensions/mine/datasets/limited.sql",
        "SELECT config('/limit') AS n",
    );
    let problems = crate::test_fixtures::activate_with_problems(&fx.svc).await;
    let limit = || async {
        serde_json::to_value(
            fx.svc
                .sql
                .query_sql("SELECT n FROM v_mine_limited", vec![], None)
                .await
                .unwrap_or_else(|e| panic!("{e}: {problems:?}"))
                .rows,
        )
        .unwrap()
    };
    let assets = crate::assets::Assets::new(fx.svc.db.clone(), Duration::from_millis(50));
    assets.sync_datasets().await.unwrap();
    assets.quiesce().await;
    // The refill is the asset's own: wait until the dataset reads the value,
    // not for the assets to look idle.
    let want = json!([[7]]);
    assert!(
        crate::test_fixtures::until(Duration::from_secs(20), || async { limit().await == want })
            .await,
        "the dataset was not refilled with {want}"
    );

    let extensions = fx.svc.extension_catalog.live_snapshot(&root);
    crate::settings::store(
        &root,
        &fx.svc.global_config_dir,
        fx.svc.secrets.clone(),
        extensions.extensions(),
    )
    .unwrap()
    .set(
        "mine.limit",
        json!(9),
        oxplow_config::settings::Level::Project,
    )
    .unwrap();
    crate::test_fixtures::activate_with_problems(&fx.svc).await;
    assets.sync_datasets().await.unwrap();
    assets.quiesce().await;
    // The refill is the asset's own: wait until the dataset reads the value,
    // not for the assets to look idle.
    let want = json!([[9]]);
    assert!(
        crate::test_fixtures::until(Duration::from_secs(20), || async { limit().await == want })
            .await,
        "the dataset was not refilled with {want}"
    );
}
