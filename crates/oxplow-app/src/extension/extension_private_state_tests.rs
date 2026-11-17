//! End to end: extensions on disk declare commands over the private state
//! and configuration operations, and they run through the real command bus.

use std::path::Path;

use oxplow_db::extension_state_store::SqliteExtensionStateStore;
use oxplow_domain::extension_state::{Area, Namespace, Scope};
use oxplow_domain::{Actor, CommandError};
use serde_json::{json, Value};

use crate::test_fixtures::{services_with_thread, ThreadFixture};

pub(crate) fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

/// An extension whose commands read and write its own private state and
/// configuration, plus a script that reads the state back.
pub(crate) fn declare(root: &Path, name: &str, configuration: &str) {
    let manifest = format!(
        "manifest: 2\nname: {name}\nintent:\n  purpose: p\n{configuration}commands:
  - name: state.write
    summary: Replace the private state.
    scope: state.write
    op: compare_exchange
  - name: state.read
    summary: Read the private state.
    scope: state.read
    op: snapshot
  - name: state.peek
    summary: Read the private state from a script.
    input_schema: {{ type: object }}
    entry: handlers/peek.star
    needs: [state.read]
  - name: config.get
    summary: Read one shared setting.
    input_schema: {{ type: object }}
    entry: handlers/config_get.star
    needs: [config.read]

"
    );
    write(
        root,
        &format!(".oxplow/extensions/{name}/extension.yaml"),
        &manifest,
    );
    write(
        root,
        &format!(".oxplow/extensions/{name}/handlers/config_get.star"),
        "def transform(x):\n    return {\"commands\": [], \"result\": scope(\"config.read\", x[\"input\"])}\n",
    );
    write(
        root,
        &format!(".oxplow/extensions/{name}/handlers/peek.star"),
        "def transform(x):\n    return {\"commands\": [], \"result\": scope(\"state.read\", {\"scope\": \"project\", \"area\": \"state\"})}\n",
    );
}

pub(crate) const CONFIGURATION: &str = "configuration:
  version: 1
  schema:
    type: object
    required: [limit]
    additionalProperties: false
    properties:
      limit: { type: integer, minimum: 1, maximum: 100, default: 10 }
      label: { type: string, default: default }
      nested:
        type: object
        properties:
          a: { type: integer, default: 1 }
          b: { type: integer, default: 1 }
";

pub(crate) async fn run(
    fx: &ThreadFixture,
    name: &str,
    input: Value,
) -> Result<Value, CommandError> {
    fx.svc
        .commands
        .run(&Actor::Human, name, input, false)
        .await
        .map(|outcome| outcome.result)
}

pub(crate) async fn reconcile(fx: &ThreadFixture) {
    crate::test_fixtures::activate(&fx.svc).await;
}

pub(crate) fn write_state(scope: &str, expected: Value, values: Value) -> Value {
    json!({ "scope": scope, "area": "state", "expected": expected,
            "schema_version": 1, "values": values })
}

#[tokio::test(flavor = "multi_thread")]
async fn a_command_writes_state_and_reads_it_back() {
    let fx = services_with_thread().await;
    declare(fx._dir.path(), "alpha", "");
    reconcile(&fx).await;

    let absent = run(
        &fx,
        "alpha.state.read",
        json!({"scope": "project", "area": "state"}),
    )
    .await
    .unwrap();
    assert_eq!(absent, Value::Null);

    let stamp = run(
        &fx,
        "alpha.state.write",
        write_state("project", Value::Null, json!({"cursor": {"page": 4}})),
    )
    .await
    .unwrap();
    assert_eq!(stamp, json!({"schema_version": 1, "revision": 1}));

    let read = run(
        &fx,
        "alpha.state.read",
        json!({"scope": "project", "area": "state"}),
    )
    .await
    .unwrap();
    assert_eq!(
        read,
        json!({"stamp": {"schema_version": 1, "revision": 1}, "values": {"cursor": {"page": 4}}})
    );

    // A script declaring `state.read` sees the same snapshot.
    let peeked = run(&fx, "alpha.state.peek", json!({})).await.unwrap();
    assert_eq!(peeked["result"], read, "{peeked}");

    // The write is audited without its values.
    let audits = fx.svc.commands.audit_store().list_recent(20).await.unwrap();
    let audit = audits
        .iter()
        .find(|a| a.command == "alpha.state.write")
        .expect("the write is audited");
    assert!(
        !format!("{audit:?}").contains("cursor"),
        "private values stay out of the audit: {audit:?}"
    );

    // A stale stamp is refused through the bus, and nothing changes.
    let stale = run(
        &fx,
        "alpha.state.write",
        write_state("project", Value::Null, json!({"cursor": 9})),
    )
    .await;
    assert!(
        matches!(stale, Err(CommandError::Invalid { .. })),
        "{stale:?}"
    );
    let next = run(
        &fx,
        "alpha.state.write",
        write_state(
            "project",
            json!({"schema_version": 1, "revision": 1}),
            json!({"cursor": 5}),
        ),
    )
    .await
    .unwrap();
    assert_eq!(next["revision"], json!(2));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_second_extension_cannot_read_the_first_ones_namespace() {
    let fx = services_with_thread().await;
    declare(fx._dir.path(), "alpha", "");
    declare(fx._dir.path(), "beta", "");
    reconcile(&fx).await;
    run(
        &fx,
        "alpha.state.write",
        write_state("project", Value::Null, json!({"secret-ish": "alpha"})),
    )
    .await
    .unwrap();

    // The same request, from beta, reads beta's own (empty) namespace.
    let beta = run(
        &fx,
        "beta.state.read",
        json!({"scope": "project", "area": "state"}),
    )
    .await
    .unwrap();
    assert_eq!(beta, Value::Null);

    // And a request cannot name another owner.
    let named = run(
        &fx,
        "beta.state.read",
        json!({"scope": "project", "area": "state", "extension": "alpha"}),
    )
    .await;
    assert!(
        matches!(named, Err(CommandError::Invalid { .. })),
        "{named:?}"
    );

    // beta's own write lands in beta's namespace only.
    run(
        &fx,
        "beta.state.write",
        write_state("project", Value::Null, json!({"mine": 1})),
    )
    .await
    .unwrap();
    let alpha = run(
        &fx,
        "alpha.state.read",
        json!({"scope": "project", "area": "state"}),
    )
    .await
    .unwrap();
    assert_eq!(alpha["values"], json!({"secret-ish": "alpha"}));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_disabled_extension_is_denied_while_its_rows_are_kept() {
    let fx = services_with_thread().await;
    declare(fx._dir.path(), "alpha", CONFIGURATION);
    reconcile(&fx).await;
    run(
        &fx,
        "alpha.state.write",
        write_state("project", Value::Null, json!({"kept": true})),
    )
    .await
    .unwrap();
    run(
        &fx,
        "oxplow.config.set",
        json!({"key":"alpha.limit", "value":7, "level":"project"}),
    )
    .await
    .unwrap();

    write(
        fx._dir.path(),
        ".oxplow/local.yaml",
        "oxplow.extensions.alpha.enabled: false\n",
    );
    reconcile(&fx).await;
    assert!(fx.svc.commands.spec("alpha.state.read").is_none());
    assert!(run(
        &fx,
        "alpha.state.read",
        json!({"scope": "project", "area": "state"})
    )
    .await
    .is_err());
    assert!(run(&fx, "alpha.config.get", json!({"key": "alpha.limit"}))
        .await
        .is_err());

    // The rows are still there for the extension's return.
    let rows = SqliteExtensionStateStore::new(fx.svc.db.clone())
        .snapshot(Namespace {
            namespace: "alpha".into(),
            scope: Scope::Project,
            area: Area::State,
        })
        .await
        .unwrap()
        .expect("disabling keeps the state row");
    assert_eq!(rows.values["kept"], json!(true));

    std::fs::remove_file(fx._dir.path().join(".oxplow/local.yaml")).unwrap();
    reconcile(&fx).await;
    let read = run(
        &fx,
        "alpha.state.read",
        json!({"scope": "project", "area": "state"}),
    )
    .await
    .unwrap();
    assert_eq!(read["values"], json!({"kept": true}));
    let config = run(&fx, "alpha.config.get", json!({"key": "alpha.limit"}))
        .await
        .unwrap();
    assert_eq!(config["result"]["value"], json!(7));
}

#[tokio::test(flavor = "multi_thread")]
async fn configuration_resolves_defaults_project_and_local() {
    let fx = services_with_thread().await;
    declare(fx._dir.path(), "alpha", CONFIGURATION);
    reconcile(&fx).await;
    let snapshot = fx
        .svc
        .extension_catalog
        .live_snapshot(&fx.svc.layout.project_dir);
    let store = crate::settings::store(
        &fx.svc.layout.project_dir,
        &fx.svc.global_config_dir,
        fx.svc.secrets.clone(),
        snapshot.extensions(),
    )
    .unwrap();
    let config = run(&fx, "alpha.config.get", json!({"key": "alpha.limit"}))
        .await
        .unwrap();
    assert_eq!(config["result"]["value"], json!(10));
    assert_eq!(config["result"]["origin"], json!("default"));
    store
        .set(
            "alpha.limit",
            json!(20),
            oxplow_config::settings::Level::Project,
        )
        .unwrap();
    store
        .set(
            "alpha.limit",
            json!(30),
            oxplow_config::settings::Level::Local,
        )
        .unwrap();
    let config = run(&fx, "alpha.config.get", json!({"key": "alpha.limit"}))
        .await
        .unwrap();
    assert_eq!(config["result"]["value"], json!(30));
    assert_eq!(config["result"]["origin"], json!("local"));
    store
        .unset("alpha.limit", oxplow_config::settings::Level::Local)
        .unwrap();
    let config = run(&fx, "alpha.config.get", json!({"key": "alpha.limit"}))
        .await
        .unwrap();
    assert_eq!(config["result"]["value"], json!(20));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_setting_failing_the_schema_is_refused_without_echoing_it() {
    let fx = services_with_thread().await;
    declare(fx._dir.path(), "alpha", CONFIGURATION);
    reconcile(&fx).await;
    for bad in [json!("hunter2-not-a-number"), json!(1000)] {
        let refused = run(
            &fx,
            "oxplow.config.set",
            json!({"key":"alpha.limit", "value":bad, "level":"project"}),
        )
        .await
        .expect_err("schema violation");
        let message = format!("{refused:?} {refused}");
        assert!(matches!(refused, CommandError::Invalid { .. }), "{message}");
        assert!(
            !message.contains("hunter2") && !message.contains("1000"),
            "{message}"
        );
    }
    let config = run(&fx, "alpha.config.get", json!({"key": "alpha.limit"}))
        .await
        .unwrap();
    assert_eq!(config["result"]["value"], json!(10));
}

#[tokio::test(flavor = "multi_thread")]
async fn state_and_configuration_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    declare(dir.path(), "alpha", CONFIGURATION);
    let db_path = dir.path().join("restart.sqlite");
    let boot = || async {
        let svc = crate::Services::on_machine_with_database(
            dir.path(),
            dir.path().join(".oxplow/global-config"),
            std::sync::Arc::new(oxplow_ai::secrets::MemorySecrets::default()),
            crate::providers::host::process_env(),
            oxplow_db::Database::open(&db_path).unwrap(),
        )
        .await
        .unwrap();
        svc
    };

    let first = boot().await;
    crate::test_fixtures::activate(&first).await;
    for (name, input) in [
        (
            "alpha.state.write",
            write_state("personal", Value::Null, json!({"kept": [1, 2]})),
        ),
        (
            "oxplow.config.set",
            json!({"key":"alpha.limit", "value":42, "level":"project"}),
        ),
    ] {
        first
            .commands
            .run(&Actor::Human, name, input, false)
            .await
            .unwrap();
    }
    drop(first);

    // A new process over the same file: nothing in memory carries over.
    let second = boot().await;
    crate::test_fixtures::activate(&second).await;
    let read = second
        .commands
        .run(
            &Actor::Human,
            "alpha.state.read",
            json!({"scope": "personal", "area": "state"}),
            false,
        )
        .await
        .unwrap()
        .result;
    assert_eq!(read["values"], json!({"kept": [1, 2]}));
    assert_eq!(read["stamp"]["revision"], json!(1));
    let config = second
        .commands
        .run(
            &Actor::Human,
            "alpha.config.get",
            json!({"key": "alpha.limit"}),
            false,
        )
        .await
        .unwrap()
        .result;
    assert_eq!(config["result"]["value"], json!(42));
    assert_eq!(config["result"]["origin"], json!("project"));
}
