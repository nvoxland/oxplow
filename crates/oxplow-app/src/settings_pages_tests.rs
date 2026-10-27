//! What no fixture can make yet of foundation's Settings pages: a language
//! server in `.oxplow/project.yaml` (a config file, not a command) and an
//! extension with collectors, whose health and credentials the Extensions
//! area lists. The other foundation pages are fixtures in the extension.

use oxplow_config::LspServerConfig;
use oxplow_db::SqlCell;

use crate::test_fixtures::services_with_effort;

type Row = Vec<SqlCell>;

struct Run {
    columns: Vec<String>,
    rows: Vec<Row>,
}

impl Run {
    fn text(&self, row: &Row, column: &str) -> Option<String> {
        let at = self.columns.iter().position(|c| c == column).unwrap();
        match &row[at] {
            SqlCell::Text(t) => Some(t.clone()),
            _ => None,
        }
    }
    fn flag(&self, row: &Row, column: &str) -> bool {
        let at = self.columns.iter().position(|c| c == column).unwrap();
        matches!(&row[at], SqlCell::Int(1))
    }
    fn keyed(&self, column: &str, value: &str) -> &Row {
        self.rows
            .iter()
            .find(|r| self.text(r, column).as_deref() == Some(value))
            .unwrap_or_else(|| panic!("no row with {column} = {value}"))
    }
    fn has(&self, column: &str, value: &str) -> bool {
        self.rows
            .iter()
            .any(|r| self.text(r, column).as_deref() == Some(value))
    }
}

async fn run(
    fx: &crate::test_fixtures::EffortFixture,
    page: &str,
    params: Vec<(&str, SqlCell)>,
) -> Run {
    let svc = &fx.svc;
    let params: Vec<(String, SqlCell)> = params
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    let run =
        crate::test_fixtures::run_as_page(svc, &format!("oxplow-foundation/{page}"), &params).await;
    Run {
        columns: run.columns,
        rows: run.rows,
    }
}

/// A server of `.oxplow/project.yaml` is listed, offers no Remove (it is the
/// file's), and the suggestions leave out its language.
#[tokio::test]
async fn a_configured_language_server_is_listed_and_not_suggested_again() {
    let fx = services_with_effort().await;
    let before = run(&fx, "lsp-suggestions", vec![]).await;
    assert!(before.has("package", "rust-analyzer"));

    fx.svc
        .config
        .write()
        .unwrap()
        .lsp_servers
        .push(LspServerConfig {
            language_id: "rust".into(),
            extensions: vec!["rs".into()],
            command: "/opt/bin/rust-analyzer".into(),
            args: vec![],
        });

    let servers = run(&fx, "lsp-servers", vec![]).await;
    let rust = servers.keyed("language", "rust");
    assert_eq!(
        servers.text(rust, "program").as_deref(),
        Some("rust-analyzer")
    );
    assert_eq!(
        servers.text(rust, "source").as_deref(),
        Some("project.yaml")
    );
    assert!(!servers.flag(rust, "can_remove"));
    assert!(!servers.flag(rust, "can_restart"));

    let after = run(&fx, "lsp-suggestions", vec![]).await;
    assert!(!after.has("package", "rust-analyzer"));
}

async fn command(
    fx: &crate::test_fixtures::EffortFixture,
    name: &str,
    input: serde_json::Value,
) -> serde_json::Value {
    fx.svc
        .commands
        .run(&oxplow_domain::Actor::Human, name, input, false)
        .await
        .unwrap()
        .result
}

/// Settings → Extensions: a collector turned off after failures says why
/// and offers Enable Again and Repair; a credential an extension's
/// collectors declare is listed once, with whether a value is kept (never
/// the value).
#[tokio::test(flavor = "multi_thread")]
async fn the_extensions_area_lists_health_and_credentials() {
    let dir = tempfile::tempdir().unwrap();
    crate::test_fixtures::init_git_repo(dir.path());
    let ext = dir.path().join(".oxplow/extensions/acme");
    std::fs::create_dir_all(&ext).unwrap();
    std::fs::write(
        ext.join("extension.yaml"),
        "manifest: 2\nname: acme\nintent:\n  purpose: test\npermissions: { process: true, secrets: [TOKEN] }\ncollectors:\n  - id: s\n    runtime: exec\n    entry: sync.sh\n    credentials: [TOKEN]\n    entities:\n      - { name: row, key: id, columns: { id: int } }\n  - id: t\n    runtime: exec\n    entry: sync.sh\n    credentials: [TOKEN]\n    entities:\n      - { name: row2, key: id, columns: { id: int } }\n",
    )
    .unwrap();
    std::fs::write(ext.join("sync.sh"), "#!/bin/sh\necho '{\"entities\":{}}'\n").unwrap();
    let fx = crate::test_fixtures::services_with_effort_in(dir).await;
    crate::test_fixtures::activate(&fx.svc).await;
    fx.svc
        .db
        .transaction(|tx| {
            oxplow_db::contribution_health_store::disable_tx(
                tx,
                &crate::collector_runner::contribution_key("acme", "s"),
                "3 failures in a row; the last: boom",
                "t",
            )
        })
        .await
        .unwrap();

    let health = run(&fx, "extensions-health", vec![]).await;
    let row = health.keyed("contribution", "s");
    assert_eq!(
        health.text(row, "status").as_deref(),
        Some("Disabled: 3 failures in a row; the last: boom")
    );
    assert!(health.flag(row, "can_enable"));
    // No open repair task yet: nothing to hand the agent.
    assert!(!health.flag(row, "can_repair"));

    let creds = run(&fx, "extensions-credentials", vec![]).await;
    assert_eq!(
        creds.rows.len(),
        1,
        "one TOKEN for the extension: {:?}",
        creds.rows
    );
    assert_eq!(
        creds.text(&creds.rows[0], "extension").as_deref(),
        Some("acme")
    );
    assert_eq!(
        creds.text(&creds.rows[0], "status").as_deref(),
        Some("Not set")
    );
    command(
        &fx,
        "oxplow.collector.set_credential",
        serde_json::json!({ "collector": creds.text(&creds.rows[0], "collector").unwrap(), "name": "TOKEN", "value": "v" }),
    )
    .await;
    let creds = run(&fx, "extensions-credentials", vec![]).await;
    assert_eq!(
        creds.text(&creds.rows[0], "status").as_deref(),
        Some("Saved in keychain")
    );
    assert!(creds.flag(&creds.rows[0], "is_set"));

    assert!(run(&fx, "extensions-secrets", vec![]).await.rows.is_empty());
}
