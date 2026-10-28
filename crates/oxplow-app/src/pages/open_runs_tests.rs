//! Open page runs: a window opens a run and hears which loaders changed;
//! a change to a dataset one of its loaders read re-runs it; closing or the
//! window going away drops it.

use std::collections::BTreeMap;

use oxplow_db::SqlCell;
use oxplow_domain::Actor;

use super::open_runs::PageRuns;
use super::runner::{RunRequest, RunTarget};
use super::test_support::fixture;
use crate::extensions::ViewContext;

fn request(page: &str, params: &[(&str, SqlCell)]) -> RunRequest {
    RunRequest {
        target: RunTarget::Page(page.into()),
        params: params
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect(),
        ctx: ViewContext::default(),
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

#[tokio::test]
async fn ordered_window_rosters_release_only_acknowledged_runs_of_their_owner() {
    let (_dir, svc) = fixture().await;
    let runs = PageRuns::default();
    let (first, _) = runs
        .open(&svc, request("acme/numbers", &[]), "one")
        .await
        .unwrap();
    let (second, _) = runs
        .open(&svc, request("acme/numbers", &[]), "one")
        .await
        .unwrap();
    let (other, _) = runs
        .open(&svc, request("acme/numbers", &[]), "two")
        .await
        .unwrap();
    // The first roster may have been sent before an open's response arrived.
    runs.report_client_runs("one", &[]);
    assert!(runs.get(&first).is_some());
    runs.report_client_runs("one", &[first.clone(), second.clone()]);
    runs.report_client_runs("two", std::slice::from_ref(&other));
    runs.report_client_runs("one", std::slice::from_ref(&second));
    assert!(runs.get(&first).is_none());
    assert!(runs.get(&second).is_some());
    assert!(runs.get(&other).is_some());
    runs.report_client_runs("one", &[]);
    assert!(runs.get(&second).is_none());
    assert!(runs.get(&other).is_some());
}

#[tokio::test]
async fn event_socket_replacement_and_disconnect_release_only_the_current_owners_runs() {
    let (_dir, svc) = fixture().await;
    let runs = PageRuns::default();
    let old = runs.connect_client("one");
    let other_connection = runs.connect_client("two");
    let (old_run, _) = runs
        .open(&svc, request("acme/numbers", &[]), "one")
        .await
        .unwrap();
    let (other, _) = runs
        .open(&svc, request("acme/numbers", &[]), "two")
        .await
        .unwrap();
    let replacement = runs.connect_client("one");
    assert!(runs.get(&old_run).is_none());
    let (current, _) = runs
        .open(&svc, request("acme/numbers", &[]), "one")
        .await
        .unwrap();
    drop(old);
    assert!(
        runs.get(&current).is_some(),
        "a stale disconnect cannot close a reconnected window"
    );
    drop(replacement);
    assert!(runs.get(&current).is_none());
    assert!(runs.get(&other).is_some());
    drop(other_connection);
    assert!(runs.get(&other).is_none());
}

#[tokio::test]
async fn expired_windows_release_runs_but_new_opens_have_time_to_report() {
    let (_dir, svc) = fixture().await;
    let runs = PageRuns::default();
    let (forgotten, _) = runs
        .open(&svc, request("acme/numbers", &[]), "gone")
        .await
        .unwrap();
    let (live, _) = runs
        .open(&svc, request("acme/numbers", &[]), "live")
        .await
        .unwrap();
    let (unacknowledged, _) = runs
        .open(&svc, request("acme/numbers", &[]), "live")
        .await
        .unwrap();
    runs.report_client_runs("live", std::slice::from_ref(&live));
    let now = oxplow_domain::Timestamp::now();
    runs.reap_expired_clients(&[], now);
    assert!(
        runs.get(&forgotten).is_some(),
        "an open can precede the window's first report"
    );
    let later = oxplow_domain::Timestamp::from_unix_ms(
        now.unix_ms() + crate::client_reports::FORGET_AFTER_MS + 1_000,
    );
    runs.reap_expired_clients(&["live".to_string()], later);
    assert!(runs.get(&forgotten).is_none());
    assert!(runs.get(&live).is_some());
    assert!(
        runs.get(&unacknowledged).is_none(),
        "a lost open reply expires even when the window keeps reporting"
    );
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

/// A dataset change re-runs the open runs that read it — only those — and
/// reports the loaders whose rows actually differ.
#[tokio::test]
async fn a_dataset_change_re_runs_the_runs_that_read_it() {
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
    let changed = runs
        .datasets_changed(&svc, &["v_dataset".to_string()])
        .await;
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

/// A dataset change re-runs only the loaders that read it and the ones
/// downstream of them: a loader that read nothing changed keeps its rows.
#[tokio::test]
async fn a_dataset_change_re_runs_only_what_reads_it() {
    let (_dir, svc) = fixture().await;
    let runs = PageRuns::default();
    let (id, before) = runs
        .open(&svc, request("acme/mixed", &[]), "w")
        .await
        .unwrap();
    runs.datasets_changed(&svc, &["v_dataset".to_string()])
        .await;
    let after = runs.get(&id).unwrap();
    let r = |run: &super::runner::PageRun, l: &str| run.loaders[l].result.rows.clone();
    assert_eq!(
        r(&before, "steady"),
        r(&after, "steady"),
        "it read nothing that changed"
    );
    assert_ne!(
        r(&before, "after"),
        r(&after, "after"),
        "downstream of `datasets`, it ran again"
    );
}

/// A read can finish before its dataset-change notification is processed.
/// Opening must register interest before that read, including catalog-only
/// invalidation, rather than waiting until it knows the loader's reads.
#[tokio::test]
async fn changes_during_the_first_read_are_observed_before_publishing_the_run() {
    use crate::commands::{Command, Handler, HandlerOutput};
    use serde_json::json;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    let (_dir, svc) = fixture().await;
    for (everything, relevant) in [(false, true), (true, true), (false, false)] {
        let runs = PageRuns::default();
        let observed = runs.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let attempts = calls.clone();
        let name = match (everything, relevant) {
            (true, _) => "oxplow.test.catalog_read",
            (false, true) => "oxplow.test.dataset_read",
            (false, false) => "oxplow.test.unrelated_read",
        };
        let mut spec = svc
            .commands
            .command("oxplow.sql.query")
            .unwrap()
            .spec
            .clone();
        spec.id = name.into();
        spec.input_schema = json!({ "type": "object" });
        spec.output_schema = None;
        spec.op = None;
        svc.commands
            .register(
                Command::new(
                    spec,
                    Handler::External(Arc::new(move |_, _| {
                        let attempt = attempts.fetch_add(1, Ordering::SeqCst) + 1;
                        // Deterministically land a change after the loader's first
                        // read, before the run can be installed. No sleeps or polling.
                        if attempt == 1 || !relevant {
                            if everything {
                                observed.notify_everything();
                            } else {
                                observed.notify_datasets(&["v_dataset".into()]);
                            }
                        }
                        Box::pin(async move {
                            Ok(HandlerOutput {
                                result: json!([{ "attempt": attempt }]),
                                ..Default::default()
                            })
                        })
                    })),
                )
                .unwrap(),
            )
            .unwrap();
        let dataset = if relevant { "v_dataset" } else { "v_page" };
        let yaml = format!("title: Opening\ndata: {{ read: {{ command: {name}, reads: [{dataset}] }} }}\nview: {{ viz: number, from: read }}\n");
        let page =
            super::schema::parse_page("acme", "acme", "opening", "pages/opening.yaml", &yaml)
                .unwrap();
        let mut req = request("acme/opening", &[]);
        req.target = RunTarget::Spec(Arc::new(page));
        let (id, run) = runs.open(&svc, req, "window").await.unwrap();
        let expected = if relevant { 2 } else { 1 };
        assert_eq!(calls.load(Ordering::SeqCst), expected);
        assert_eq!(
            run.loaders["read"].result.rows,
            [vec![SqlCell::Int(expected as i64)]]
        );
        assert_eq!(runs.get(&id), Some(run));
    }
}

#[tokio::test]
async fn a_slow_dataset_refresh_cannot_restore_older_params_after_a_newer_request() {
    use crate::commands::{Command, Handler, HandlerOutput};
    use serde_json::json;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    let (_dir, svc) = fixture().await;
    let runs = PageRuns::default();
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let started = entered.clone();
    let gate = release.clone();
    let mut spec = svc
        .commands
        .command("oxplow.sql.query")
        .unwrap()
        .spec
        .clone();
    spec.id = "oxplow.test.delayed_read".into();
    spec.input_schema = json!({ "type": "object" });
    spec.output_schema = None;
    spec.op = None;
    svc.commands
        .register(
            Command::new(
                spec,
                Handler::External(Arc::new(move |_, input| {
                    let second = calls.fetch_add(1, Ordering::SeqCst) == 1;
                    let started = started.clone();
                    let gate = gate.clone();
                    Box::pin(async move {
                        if second {
                            started.notify_one();
                            gate.notified().await;
                        }
                        Ok(HandlerOutput {
                            result: json!([{ "n": input["n"] }]),
                            ..Default::default()
                        })
                    })
                })),
            )
            .unwrap(),
        )
        .unwrap();
    let page = super::schema::parse_page("acme", "acme", "delayed", "pages/delayed.yaml",
        "title: Delayed\nparams: [{ name: n, default: 0 }]\ndata:\n  read:\n    command: oxplow.test.delayed_read\n    args: { n: { param: n } }\n    reads: [v_dataset]\nview: { viz: number, from: read }\n").unwrap();
    let mut req = request("acme/delayed", &[]);
    req.target = RunTarget::Spec(Arc::new(page));
    let (id, _) = runs.open(&svc, req, "window").await.unwrap();
    let refreshing = runs.clone();
    let services = svc.clone();
    let pending = tokio::spawn(async move {
        refreshing
            .datasets_changed(&services, &["v_dataset".into()])
            .await
    });
    entered.notified().await;
    let (newer, _) = runs
        .set_params(&svc, &id, BTreeMap::from([("n".into(), SqlCell::Int(3))]))
        .await
        .unwrap();
    release.notify_one();
    assert!(
        pending.await.unwrap().is_empty(),
        "a stale refresh publishes no change"
    );
    assert_eq!(newer.params["n"], SqlCell::Int(3));
    assert_eq!(runs.get(&id), Some(newer));
    let (refreshed, _) = runs.rerun(&svc, &id).await.unwrap();
    assert_eq!(refreshed.params["n"], SqlCell::Int(3));
}

/// A live dataset changes with an event, not a table (a keychain write is
/// `CredentialChanged`, a config edit `ConfigChanged`): the event re-runs
/// the open runs that read the dataset, as a table's change does.
#[tokio::test]
async fn an_event_a_live_dataset_follows_re_runs_what_reads_it() {
    use crate::commands::{Command, Handler, HandlerOutput};
    use serde_json::json;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    let (_dir, svc) = fixture().await;
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let mut spec = svc
        .commands
        .command("oxplow.sql.query")
        .unwrap()
        .spec
        .clone();
    spec.id = "oxplow.test.config_read".into();
    spec.input_schema = json!({ "type": "object" });
    spec.output_schema = None;
    spec.op = None;
    svc.commands
        .register(
            Command::new(
                spec,
                Handler::External(Arc::new(move |_, _| {
                    let n = counted.fetch_add(1, Ordering::SeqCst) + 1;
                    Box::pin(async move {
                        Ok(HandlerOutput {
                            result: json!([{ "n": n }]),
                            ..Default::default()
                        })
                    })
                })),
            )
            .unwrap(),
        )
        .unwrap();
    let yaml = "title: Config\ndata: { read: { command: oxplow.test.config_read, reads: [v_config_value] } }\nview: { viz: number, from: read }\n";
    let page =
        super::schema::parse_page("acme", "acme", "config", "pages/config.yaml", yaml).unwrap();
    let mut req = request("acme/config", &[]);
    req.target = RunTarget::Spec(Arc::new(page));
    let runs = svc.page_runs.clone();
    runs.start(&svc);
    let mut ui = svc.events.subscribe_ui();
    let (id, _) = runs.open(&svc, req, "window").await.unwrap();

    svc.events.emit(crate::events::OxplowEvent::ConfigChanged);
    let heard = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if let crate::events::OxplowEvent::PageRunChanged(change) = ui.recv().await.unwrap() {
                if change.run_id == id {
                    return change;
                }
            }
        }
    })
    .await
    .expect("the run re-ran");
    assert_eq!(heard.loaders, ["read"]);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}
