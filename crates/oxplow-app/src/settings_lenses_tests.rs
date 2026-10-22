//! Settings' Overview and Language Servers areas are oxplow-foundation lenses
//! mounted in each area's slot: what they list, how they group and search,
//! what Ask puts in the agent's input, and what the language-server rows offer.

use oxplow_config::LspServerConfig;
use oxplow_db::SqlCell;

use crate::extensions::{run_lens, LensContext};
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
    lens: &str,
    params: Vec<(&str, SqlCell)>,
) -> Run {
    let svc = &fx.svc;
    let run = run_lens(
        &svc.sql,
        &svc.extension_catalog,
        fx._dir.path(),
        &format!("oxplow-foundation/{lens}"),
        params
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
        &LensContext::default(),
    )
    .await
    .unwrap();
    Run {
        columns: run.result.columns,
        rows: run.result.rows,
    }
}

fn q(text: &str) -> Vec<(&str, SqlCell)> {
    vec![("q", SqlCell::Text(text.to_string()))]
}

/// Every setting is a row under its group, with where its value comes from
/// and the request Ask puts in the agent's input.
#[tokio::test]
async fn every_setting_is_listed_by_group_with_its_source_and_ask() {
    let fx = services_with_effort().await;
    let all = run(&fx, "settings-values", q("")).await;
    assert!(!all.rows.is_empty());

    let agents = all.keyed("key", "agents");
    assert_eq!(all.text(agents, "grp").as_deref(), Some("Agents"));
    // Only the person sets it, and its control is on the Agents area's page.
    assert_eq!(all.text(agents, "who").as_deref(), Some("yours to set"));
    assert_eq!(
        all.text(agents, "control_page").as_deref(),
        Some("oxplow:oxplow/page?area=agents&page=settings")
    );

    let retention = all.keyed("key", "snapshotRetentionDays");
    assert_eq!(all.text(retention, "grp").as_deref(), Some("Snapshots"));
    assert_eq!(all.text(retention, "source").as_deref(), Some("default"));
    let ask = all.text(retention, "ask").unwrap();
    assert!(
        ask.starts_with("Change the setting `snapshotRetentionDays` — "),
        "{ask}"
    );
    assert!(ask.contains("(the default)"), "{ask}");
    assert!(ask.contains("oxplow.config.set"), "{ask}");

    // Groups come in a fixed order: Agents before Snapshots.
    let groups: Vec<String> = all
        .rows
        .iter()
        .map(|r| all.text(r, "grp").unwrap())
        .collect();
    let first_snapshots = groups.iter().position(|g| g == "Snapshots").unwrap();
    let first_agents = groups.iter().position(|g| g == "Agents").unwrap();
    assert!(first_agents < first_snapshots);
}

/// A search matches a setting's key, its doc or its whole value.
#[tokio::test]
async fn the_search_narrows_by_key_doc_or_value() {
    let fx = services_with_effort().await;
    let found = run(&fx, "settings-values", q("snapshotretention")).await;
    assert!(found.has("key", "snapshotRetentionDays"));
    assert!(!found.has("key", "agents"));
    let none = run(&fx, "settings-values", q("no such setting anywhere")).await;
    assert!(none.rows.is_empty());
    // No search (NULL) lists all.
    let all = run(&fx, "settings-values", vec![("q", SqlCell::Null(()))]).await;
    assert!(all.rows.len() > found.rows.len());
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
