//! Open page runs: a window opens a run and hears which loaders changed;
//! a change to a model one of its loaders read re-runs it; closing or the
//! window going away drops it.

use std::collections::BTreeMap;

use oxplow_db::SqlCell;
use oxplow_domain::Actor;

use super::open_runs::PageRuns;
use super::runner::{RunRequest, RunTarget};
use super::test_support::fixture;
use crate::extensions::LensContext;

fn request(page: &str, params: &[(&str, SqlCell)]) -> RunRequest {
    RunRequest {
        target: RunTarget::Page(page.into()),
        params: params
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
        ctx: LensContext::default(),
        viewer: Actor::Human,
    }
}

#[tokio::test]
async fn an_open_run_is_kept_until_it_is_closed() {
    let (_dir, svc) = fixture().await;
    let runs = PageRuns::default();
    let (id, run) = runs
        .open(&svc, request("acme/numbers", &[]), "window-1")
        .await
        .unwrap();
    assert_eq!(run.page.id, "acme/numbers");
    assert_eq!(
        runs.get(&id).map(|r| r.page.id),
        Some("acme/numbers".into())
    );
    runs.close(&id);
    assert!(runs.get(&id).is_none());
}

/// A param change (a pick, a filter, a switch) re-runs it with the new
/// values, and says which loaders' rows changed.
#[tokio::test]
async fn new_params_re_run_it_and_name_what_changed() {
    let (_dir, svc) = fixture().await;
    let runs = PageRuns::default();
    let (id, _) = runs
        .open(&svc, request("acme/numbers", &[]), "w")
        .await
        .unwrap();
    let (run, changed) = runs
        .set_params(
            &svc,
            &id,
            BTreeMap::from([("n".to_string(), SqlCell::Int(3))]),
        )
        .await
        .unwrap();
    assert_eq!(run.params.get("n"), Some(&SqlCell::Int(3)));
    assert_eq!(changed.loaders, ["big", "count"]);
    assert!(!changed.tree);
}

/// A model change re-runs the open runs that read it — only those — and
/// reports the loaders whose rows actually differ.
#[tokio::test]
async fn a_model_change_re_runs_the_runs_that_read_it() {
    let (_dir, svc) = fixture().await;
    let runs = PageRuns::default();
    let (reads, _) = runs
        .open(&svc, request("acme/queried", &[]), "w")
        .await
        .unwrap();
    let (other, _) = runs
        .open(&svc, request("acme/numbers", &[]), "w")
        .await
        .unwrap();
    let changed = runs.models_changed(&svc, &["v_model".to_string()]).await;
    let ids: Vec<&str> = changed.iter().map(|c| c.run_id.as_str()).collect();
    assert!(!ids.contains(&other.as_str()), "{ids:?}");
    // It re-ran; its rows are the same, so nothing is reported for it.
    assert!(changed
        .iter()
        .all(|c| c.run_id != reads || c.loaders.is_empty()));
}

/// A window's runs go when its client does.
#[tokio::test]
async fn a_clients_runs_go_with_it() {
    let (_dir, svc) = fixture().await;
    let runs = PageRuns::default();
    let (a, _) = runs
        .open(&svc, request("acme/numbers", &[]), "gone")
        .await
        .unwrap();
    let (b, _) = runs
        .open(&svc, request("acme/numbers", &[]), "here")
        .await
        .unwrap();
    runs.keep_only_clients(&["here".to_string()]);
    assert!(runs.get(&a).is_none());
    assert!(runs.get(&b).is_some());
}
