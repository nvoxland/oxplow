//! A page's actions run by the open run they were pressed on: a whole-page
//! action with the run's params, a row action on a row the run holds — the
//! server binds both from its own copy of the run.

use std::collections::BTreeMap;

use oxplow_db::SqlCell;
use oxplow_domain::{Actor, CommandError};

use super::actions::{page_form, run_page_action, submit_page_form, PageActionCall};
use super::open_runs::PageRuns;
use super::runner::{RunRequest, RunTarget};
use super::test_support::fixture;
use crate::extensions::LensContext;

async fn titles(svc: &crate::Services) -> Vec<String> {
    let r = svc
        .sql
        .run(oxplow_db::SqlQuery::new(
            "SELECT title FROM v_work_item ORDER BY title",
        ))
        .await
        .unwrap();
    r.rows
        .iter()
        .map(|row| match &row[0] {
            SqlCell::Text(t) => t.clone(),
            other => panic!("{other:?}"),
        })
        .collect()
}

fn call(
    run_id: &str,
    node: Option<&str>,
    action: &str,
    row: Option<&[(&str, &str)]>,
) -> PageActionCall {
    PageActionCall {
        run_id: run_id.into(),
        node: node.map(str::to_string),
        action: action.into(),
        row: row.map(|r| {
            r.iter()
                .map(|(k, v)| (k.to_string(), SqlCell::Text(v.to_string())))
                .collect::<BTreeMap<_, _>>()
        }),
        input: None,
        drop: None,
        on_behalf_of: Actor::Human,
        confirmed: false,
    }
}

#[tokio::test]
async fn a_page_action_runs_with_the_runs_params_and_a_row_action_on_its_row() {
    let (_dir, svc) = fixture().await;
    let runs = PageRuns::default();
    let (id, _) = runs
        .open(
            &svc,
            RunRequest {
                target: RunTarget::Page("acme/acts".into()),
                params: BTreeMap::new(),
                ctx: LensContext::default(),
                viewer: Actor::Human,
            },
            "w",
        )
        .await
        .unwrap();
    run_page_action(&svc, &runs, call(&id, None, "add", None))
        .await
        .unwrap();
    run_page_action(
        &svc,
        &runs,
        call(&id, Some("names"), "add-row", Some(&[("name", "beta")])),
    )
    .await
    .unwrap();
    assert_eq!(titles(&svc).await, ["beta", "from-page"]);
    // A row the run doesn't hold is refused: the page changed under it.
    let err = run_page_action(
        &svc,
        &runs,
        call(&id, Some("names"), "add-row", Some(&[("name", "gamma")])),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&err, CommandError::Invalid { message, .. } if message.contains("isn't one of")),
        "{err:?}"
    );
    // An unknown run says so.
    let err = run_page_action(&svc, &runs, call("nope", None, "add", None))
        .await
        .unwrap_err();
    assert!(matches!(err, CommandError::Invalid { .. }), "{err:?}");
}

/// A whole-page action that reads a row runs on the page's first loader's
/// one row, which the server reads: a document's row, as it was.
#[tokio::test]
async fn a_page_action_reads_the_first_loaders_row() {
    let (_dir, svc) = fixture().await;
    let runs = PageRuns::default();
    let (id, _) = runs
        .open(
            &svc,
            RunRequest {
                target: RunTarget::Page("acme/session".into()),
                params: BTreeMap::new(),
                ctx: LensContext::default(),
                viewer: Actor::Human,
            },
            "w",
        )
        .await
        .unwrap();
    run_page_action(&svc, &runs, call(&id, None, "stop", None))
        .await
        .unwrap();
    assert_eq!(titles(&svc).await, ["stop sess-1"]);
}

/// A form node starts from its defaults (params bound) with its choices
/// read from a page's loader, and submits its command for the person.
#[tokio::test]
async fn a_form_node_starts_from_its_defaults_and_submits() {
    let (_dir, svc) = fixture().await;
    let runs = PageRuns::default();
    let (id, _) = runs
        .open(
            &svc,
            RunRequest {
                target: RunTarget::Page("acme/new-item".into()),
                params: BTreeMap::new(),
                ctx: LensContext::default(),
                viewer: Actor::Human,
            },
            "w",
        )
        .await
        .unwrap();
    let start = page_form(&svc, &runs, &id, "view").await.unwrap();
    assert_eq!(start.command.id, "oxplow.work_item.create");
    assert_eq!(start.values["title"], serde_json::json!("drafted"));
    let states: Vec<&str> = start.choices["state"]
        .iter()
        .map(|c| c.value.as_str())
        .collect();
    assert_eq!(states, ["todo", "done"]);
    submit_page_form(
        &svc,
        &runs,
        &id,
        "view",
        serde_json::json!({ "title": "submitted" }),
        Actor::Human,
        false,
    )
    .await
    .unwrap();
    assert_eq!(titles(&svc).await, ["submitted"]);
}
