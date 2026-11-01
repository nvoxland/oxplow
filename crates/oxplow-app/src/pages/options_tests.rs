//! A choice's options come from a query, or from a loader of the page
//! itself: only that loader's chain runs, with the params it declares.

use std::collections::BTreeMap;

use oxplow_db::SqlCell;
use oxplow_domain::Actor;

use super::options::{param_options, source_rows};
use super::test_support::fixture;
use crate::pages::spec::ViewContext;

fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, SqlCell> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), SqlCell::Text(v.to_string())))
        .collect()
}

fn texts(rows: &[Vec<SqlCell>], col: usize) -> Vec<String> {
    rows.iter()
        .map(|r| match &r[col] {
            SqlCell::Text(t) => t.clone(),
            other => format!("{other:?}"),
        })
        .collect()
}

/// A param's options come from the query its `options_from` names, run
/// with only the params that query declares (the host's `end` and `other`
/// never reach it), as rows of `value` and `label`.
#[tokio::test]
async fn a_params_options_come_from_its_query_with_the_params_it_declares() {
    let (_d, svc) = fixture().await;
    let rows = param_options(
        &svc,
        "acme/chooser",
        "end",
        &values(&[("start", "snap:1"), ("end", "x"), ("other", "1")]),
        &ViewContext::default(),
        &Actor::Human,
    )
    .await
    .unwrap();
    assert_eq!(rows.columns, ["value", "label"]);
    assert_eq!(texts(&rows.rows, 0), ["snap:2"]);
}

/// `options_from: '#modes'` names a loader of the page itself; the rail
/// gets that loader's rows without running the rest of the page.
#[tokio::test]
async fn a_params_options_come_from_its_own_loader() {
    let (_d, svc) = fixture().await;
    let rows = param_options(
        &svc,
        "acme/chooser",
        "mode",
        &values(&[]),
        &ViewContext::default(),
        &Actor::Human,
    )
    .await
    .unwrap();
    assert_eq!(texts(&rows.rows, 0), ["a", "b"]);
    // The same reference, read as a form's choices would read it.
    let rows = source_rows(
        &svc,
        "acme/chooser#modes",
        &values(&[]),
        &ViewContext::default(),
        &Actor::Human,
    )
    .await
    .unwrap();
    assert_eq!(texts(&rows.rows, 1), ["A", "B"]);
}

/// A source whose rows have no `value` and `label` is refused, naming
/// what it lacks; so is a loader the target doesn't have.
#[tokio::test]
async fn a_source_without_value_and_label_is_refused() {
    let (_d, svc) = fixture().await;
    let err = param_options(
        &svc,
        "acme/badchooser",
        "pick",
        &values(&[]),
        &ViewContext::default(),
        &Actor::Human,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("`acme/plain`: the rows lack `value`, `label`"),
        "{err}"
    );
    assert!(err.contains("columns: v"), "{err}");
    let err = source_rows(
        &svc,
        "acme/chooser#nope",
        &values(&[]),
        &ViewContext::default(),
        &Actor::Human,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(err.contains("`acme/chooser` has no loader `nope`"), "{err}");
}
