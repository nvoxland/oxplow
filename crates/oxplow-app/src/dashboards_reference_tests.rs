//! The examples in the `oxplow-extension` skill's `dashboards` reference,
//! run as written: the dashboard it builds is built, tile by tile, and the
//! component's lens query runs. The examples are read out of the reference
//! file, so the text and these checks cannot drift.

use serde_json::{json, Value};

use crate::test_fixtures::services_with_effort;

const REFERENCE: &str =
    include_str!("../../oxplow-agent-text/assets/skills/oxplow-extension/references/dashboards.md");

/// Every fenced block of `lang` in the reference, in order.
fn blocks(lang: &str) -> Vec<String> {
    REFERENCE
        .split(&format!("```{lang}\n"))
        .skip(1)
        .map(|rest| {
            rest.split_once("```")
                .expect("the block closes")
                .0
                .to_string()
        })
        .collect()
}

/// The build-health example builds its dashboard: each call runs, as an
/// agent, and every tile it adds is saved.
#[tokio::test]
async fn the_build_health_dashboard_builds() {
    let fx = services_with_effort().await;
    let agent = oxplow_domain::Actor::Agent {
        session_id: None,
        thread_id: Some(fx.thread),
        stream_id: None,
    };
    let calls: Vec<Value> = blocks("json")
        .iter()
        .map(|b| serde_json::from_str(b).unwrap_or_else(|e| panic!("{e}: {b}")))
        .collect();
    let (create, tiles) = calls.split_first().expect("the example has calls");
    assert_eq!(create["name"], "oxplow.dashboard.create");
    let made = fx
        .svc
        .commands
        .run(
            &agent,
            "oxplow.dashboard.create",
            create["input"].clone(),
            false,
        )
        .await
        .unwrap();
    let dashboard = made.result["id"].as_str().expect("an id").to_string();
    for call in tiles {
        assert_eq!(call["name"], "oxplow.dashboard.add_item");
        let mut input = call["input"].clone();
        input["dashboard"] = json!(dashboard);
        fx.svc
            .commands
            .run(&agent, "oxplow.dashboard.add_item", input, false)
            .await
            .unwrap_or_else(|e| panic!("{e}: {call}"));
    }
    let saved = fx
        .svc
        .sql
        .query_sql("SELECT count(*) FROM v_dashboard_item", vec![], None)
        .await
        .unwrap();
    assert_eq!(
        saved.rows[0][0],
        oxplow_db::SqlCell::Int(tiles.len() as i64)
    );
}

/// The component example's lens query runs.
#[tokio::test]
async fn the_component_lens_query_runs() {
    let fx = services_with_effort().await;
    let lens = blocks("yaml")
        .into_iter()
        .find(|b| b.contains("viz: custom"))
        .expect("the example has a custom lens");
    let query = lens
        .lines()
        .find_map(|l| l.strip_prefix("query: "))
        .expect("its query");
    fx.svc
        .sql
        .query_sql(query, vec![], None)
        .await
        .unwrap_or_else(|e| panic!("{e}: {query}"));
}
