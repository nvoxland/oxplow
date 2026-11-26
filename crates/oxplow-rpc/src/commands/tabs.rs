//! Cores for the `tabs` command module — a window reports the tabs it
//! shows (per thread, which is selected, the thread it shows, its focus
//! and visibility, and its active page's detail), so an agent can see what
//! the person sees (`oxplow_list_tabs`, `oxplow_get_open_page`). Held in
//! memory (`oxplow_app::client_reports`).

use oxplow_app::client_reports::{TabReport, TabReportAnswer};
use oxplow_db::tab_session_store::{TabRevision, TabSave, TabSession};
use oxplow_domain::{ThreadId, Timestamp};

use crate::error::IpcError;
use crate::RpcRequest;

/// Take `report` as what the page load asking shows now.
pub async fn report_tabs(
    req: &RpcRequest<'_>,
    report: TabReport,
) -> Result<TabReportAnswer, IpcError> {
    let caller = req.caller()?;
    let missing_runs = req
        .page_runs
        .report_client_runs(caller.connection.as_str(), &report.runs);
    req.client_reports.report(caller, report, Timestamp::now());
    Ok(TabReportAnswer { missing_runs })
}

/// The caller device's tabs of every thread it opened (none for a request
/// with no device: it has no tabs of its own).
pub async fn list_tab_sessions(req: &RpcRequest<'_>) -> Result<Vec<TabSession>, IpcError> {
    match req.caller {
        Some(caller) => Ok(req.tab_session_store.list(&caller.device).await?),
        None => Ok(Vec::new()),
    }
}

/// Keep `session_json` as the caller device's tabs of `thread_id`, if they
/// are still at `revision` (the one it read; `None` for a thread it has no
/// tabs of yet). When they moved on since, nothing is written and the
/// answer says the revision they are at.
#[expect(
    clippy::disallowed_methods,
    reason = "off the bus: a device's tabs of a thread"
)]
pub async fn save_tab_session(
    req: &RpcRequest<'_>,
    thread_id: ThreadId,
    session_json: String,
    revision: Option<TabRevision>,
) -> Result<TabSave, IpcError> {
    let device = &req.caller()?.device;
    Ok(req
        .tab_session_store
        .save(device, &thread_id, session_json, revision)
        .await?)
}

/// Forget the caller device's tabs of `thread_id`: the thread opens on it as
/// one it never opened.
#[expect(
    clippy::disallowed_methods,
    reason = "off the bus: a device's tabs of a thread"
)]
pub async fn forget_tab_session(req: &RpcRequest<'_>, thread_id: ThreadId) -> Result<(), IpcError> {
    let device = &req.caller()?.device;
    req.tab_session_store.forget(device, &thread_id).await?;
    Ok(())
}

/// The tabs agents opened for the caller device while it was away, oldest
/// first, each with the thread it opens in; they are gone from the queue
/// once read. A request with no device has none.
#[expect(
    clippy::disallowed_methods,
    reason = "off the bus: the tabs an agent opened for a device that was away"
)]
pub async fn take_queued_tabs(req: &RpcRequest<'_>) -> Result<Vec<QueuedTab>, IpcError> {
    let Some(caller) = req.caller else {
        return Ok(Vec::new());
    };
    Ok(req
        .tab_queue_store
        .take(&caller.device)
        .await?
        .into_iter()
        .map(|(thread_id, uri)| QueuedTab { thread_id, uri })
        .collect())
}

/// A tab waiting for a device: the page, and the thread it opens in.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct QueuedTab {
    pub thread_id: ThreadId,
    pub uri: String,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    /// A device takes the tabs agents opened for it while it was away, once;
    /// another device, or a request with none, gets none.
    #[tokio::test]
    async fn a_device_takes_the_tabs_opened_for_it() {
        let (svc, _dir) = crate::test_support::services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let thread = svc
            .threads
            .selected_or_first(None, &stream.id)
            .await
            .unwrap()
            .unwrap();
        let mac = crate::test_support::caller();
        let tablet = oxplow_domain::Caller {
            device: oxplow_domain::DeviceId::generate(),
            connection: oxplow_domain::ConnectionId::generate(),
        };
        // An agent shows a page on the tablet, which is away.
        let shown = svc
            .commands
            .run(
                &oxplow_domain::Actor::Agent {
                    session_id: None,
                    thread_id: Some(thread),
                    stream_id: None,
                },
                "oxplow.page.show",
                json!({
                    "spec": "title: Waiting\nloaders:\n  n: { sql: \"SELECT 1 AS n\" }\nview: { id: n, viz: number, from: n }\n",
                    "client": tablet.device.to_string(),
                }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(shown.result["queued"], json!(true));
        let uri = shown.result["uri"].clone();
        let take = |caller: Option<&oxplow_domain::Caller>| {
            let svc = svc.clone();
            let caller = caller.cloned();
            async move {
                crate::dispatch_as("take_queued_tabs", json!({}), &svc, caller.as_ref())
                    .await
                    .unwrap()
            }
        };
        assert_eq!(take(Some(&mac)).await, json!([]));
        assert_eq!(take(None).await, json!([]));
        assert_eq!(
            take(Some(&tablet)).await,
            json!([{ "threadId": thread.to_string(), "uri": uri }])
        );
        assert_eq!(take(Some(&tablet)).await, json!([]), "taken once");
    }

    /// A device's tabs read back as it saved them; another device, or a
    /// request with none, has none; a save that read an old revision is
    /// refused; a forgotten thread's tabs go.
    #[tokio::test]
    async fn a_devices_tabs_are_its_own() {
        let (svc, _dir) = crate::test_support::services().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let thread = svc
            .threads
            .selected_or_first(None, &stream.id)
            .await
            .unwrap()
            .unwrap()
            .to_string();
        let mac = crate::test_support::caller();
        let tablet = oxplow_domain::Caller {
            device: oxplow_domain::DeviceId::generate(),
            connection: oxplow_domain::ConnectionId::generate(),
        };
        let ask = |name: &'static str,
                   args: serde_json::Value,
                   caller: Option<&oxplow_domain::Caller>| {
            let svc = svc.clone();
            let caller = caller.cloned();
            async move { crate::dispatch_as(name, args, &svc, caller.as_ref()).await }
        };
        let save = |revision: serde_json::Value| json!({ "threadId": thread, "sessionJson": "{\"tabs\":[]}", "revision": revision });
        let first = ask(
            "save_tab_session",
            save(serde_json::Value::Null),
            Some(&mac),
        )
        .await
        .unwrap();
        assert_eq!(first, json!({ "saved": true, "revision": 1 }));
        assert_eq!(
            ask("save_tab_session", save(json!(1)), Some(&mac))
                .await
                .unwrap(),
            json!({ "saved": true, "revision": 2 })
        );
        assert_eq!(
            ask("save_tab_session", save(json!(1)), Some(&mac))
                .await
                .unwrap(),
            json!({ "saved": false, "revision": 2 }),
            "a save that read an old revision is refused, saying where the row is"
        );
        let listed = ask("list_tab_sessions", json!({}), Some(&mac))
            .await
            .unwrap();
        assert_eq!(
            listed,
            json!([{ "threadId": thread, "sessionJson": "{\"tabs\":[]}", "revision": 2 }])
        );
        assert_eq!(
            ask("list_tab_sessions", json!({}), Some(&tablet))
                .await
                .unwrap(),
            json!([])
        );
        assert_eq!(
            ask("list_tab_sessions", json!({}), None).await.unwrap(),
            json!([])
        );
        assert_eq!(
            ask("save_tab_session", save(serde_json::Value::Null), None)
                .await
                .unwrap_err()
                .code,
            "INVALID"
        );
        ask(
            "forget_tab_session",
            json!({ "threadId": thread }),
            Some(&mac),
        )
        .await
        .unwrap();
        assert_eq!(
            ask("list_tab_sessions", json!({}), Some(&mac))
                .await
                .unwrap(),
            json!([])
        );
    }

    /// A window's report reads back as its threads' tabs, as its page load
    /// and device; a malformed thread, or a report with no caller, is refused.
    #[tokio::test]
    async fn a_windows_report_is_read_back() {
        let (svc, _dir) = crate::test_support::services().await;
        let caller = crate::test_support::caller();
        let report = || {
            json!({ "report": {
                "focused": true, "visible": true, "shown_thread": "thr1",
                "threads": [{ "thread": "thr1", "active": "oxplow:oxplow/knowledge?id=home&store=oxwiki",
                              "tabs": [{ "id": "oxplow:oxplow/knowledge?id=home&store=oxwiki", "kind": "oxplow/knowledge", "title": "Home" }] }],
                "detail_json": null
            } })
        };
        crate::dispatch_as("report_tabs", report(), &svc, Some(&caller))
            .await
            .unwrap();
        let t = oxplow_domain::ThreadId::try_from_str("thr1").unwrap();
        let pages = svc
            .client_reports
            .open_pages(t, oxplow_domain::Timestamp::now());
        assert_eq!(
            pages[0].page_id,
            "oxplow:oxplow/knowledge?id=home&store=oxwiki"
        );
        assert_eq!(pages[0].client, caller.connection.as_str());
        assert_eq!(pages[0].device, caller.device.as_str());

        let anonymous = crate::dispatch("report_tabs", report(), &svc)
            .await
            .unwrap_err();
        assert_eq!(anonymous.code, "INVALID");

        let err = crate::dispatch_as(
            "report_tabs",
            json!({ "report": { "focused": true, "visible": true, "shown_thread": "nope",
                                "threads": [], "detail_json": null } }),
            &svc,
            Some(&caller),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID");
    }
}
