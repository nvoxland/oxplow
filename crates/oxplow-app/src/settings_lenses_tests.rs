//! Foundation lenses that replaced core pages: Settings' Overview and Language
//! Servers areas (mounted in each area's slot: what they list, how they group
//! and search, what Ask puts in the agent's input, what the language-server
//! rows offer) and the Comments inbox.

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

fn threads_params(stream: i64, days: &str) -> Vec<(&'static str, SqlCell)> {
    vec![
        ("stream_id", SqlCell::Int(stream)),
        ("days", SqlCell::Text(days.into())),
    ]
}

/// The inbox lists open threads grouped by what they're anchored to; a
/// resolved one only when the filter reaches back to it; a picked thread
/// reads as its messages in order.
#[tokio::test]
async fn the_comments_inbox_lists_threads_by_target_and_a_picked_one_reads_in_order() {
    let fx = services_with_effort().await;
    let stream = fx.svc.streams.ensure_primary().await.unwrap().id;
    let add = |quote: &'static str, intent: &'static str| {
        command(
            &fx,
            "oxplow.knowledge.add_comment",
            serde_json::json!({
                "stream": stream.to_string(),
                "target": "oxplow:oxplow/wiki?slug=a-page",
                "quote": quote,
                "intent": intent,
                "body": "first",
            }),
        )
    };
    let first = add("one", "note").await;
    let second = add("two", "followup").await;
    let id = |c: &serde_json::Value| c["comment"]["id"].as_str().unwrap().to_string();
    let (first, second) = (id(&first), id(&second));

    let open = run(&fx, "comments-threads", threads_params(stream.value(), "0")).await;
    assert_eq!(open.rows.len(), 2);
    assert!(open.has("quote", "one") && open.has("quote", "two"));
    let row = open.keyed("quote", "two");
    assert_eq!(open.text(row, "kind").as_deref(), Some("follow-up"));
    assert_eq!(
        open.text(row, "target_label").as_deref(),
        Some("wiki/a-page")
    );
    assert_eq!(open.text(row, "go").as_deref(), Some("Go to location"));
    assert_eq!(open.text(row, "latest").as_deref(), Some("user: first"));
    assert!(open.flag(row, "is_open") && open.flag(row, "is_followup"));

    // Resolving hides it until the filter reaches back to it.
    command(
        &fx,
        "oxplow.knowledge.update_comment",
        serde_json::json!({ "comment": first, "status": "resolved" }),
    )
    .await;
    let after = run(&fx, "comments-threads", threads_params(stream.value(), "0")).await;
    assert_eq!(after.rows.len(), 1);
    let week = run(&fx, "comments-threads", threads_params(stream.value(), "7")).await;
    assert_eq!(week.rows.len(), 2);
    let resolved = week.keyed("quote", "one");
    assert_eq!(week.text(resolved, "state").as_deref(), Some("resolved"));
    assert!(week.flag(resolved, "is_resolved"));

    // A picked thread: its messages in the order they were written.
    command(
        &fx,
        "oxplow.knowledge.reply_comment",
        serde_json::json!({ "comment": second, "body": "second" }),
    )
    .await;
    let conversation = run(
        &fx,
        "comments-conversation",
        vec![("comment", SqlCell::Text(second.clone()))],
    )
    .await;
    let bodies: Vec<String> = conversation
        .rows
        .iter()
        .map(|r| conversation.text(r, "body").unwrap())
        .collect();
    assert_eq!(bodies, vec!["first", "second"]);
    // Nothing picked, nothing shown.
    let none = run(
        &fx,
        "comments-conversation",
        vec![("comment", SqlCell::Null(()))],
    )
    .await;
    assert!(none.rows.is_empty());
}
