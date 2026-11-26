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

/// Another device's stored tabs of a thread: which device, its name when
/// the daemon knows it, and its session as it keeps it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct OtherDeviceTabs {
    pub device: String,
    pub name: Option<String>,
    pub session_json: String,
}

/// The other devices' tabs of `thread_id` (Continue from <device>): every
/// device's but the caller's, the one the person used last first, then by
/// id.
pub async fn list_other_device_tabs(
    req: &RpcRequest<'_>,
    thread_id: ThreadId,
) -> Result<Vec<OtherDeviceTabs>, IpcError> {
    let mine = req.caller.map(|c| c.device.clone());
    let used = req.client_devices.last_used().map(|(d, _)| d);
    let mut out: Vec<(bool, OtherDeviceTabs)> = req
        .tab_session_store
        .list_all()
        .await?
        .into_iter()
        .filter(|(device, row)| row.thread_id == thread_id && Some(device) != mine.as_ref())
        .map(|(device, row)| {
            (
                Some(&device) == used.as_ref(),
                OtherDeviceTabs {
                    name: req.client_devices.get(&device).map(|d| d.name),
                    device: device.as_str().to_string(),
                    session_json: row.session_json,
                },
            )
        })
        .collect();
    out.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.device.cmp(&b.1.device)));
    Ok(out.into_iter().map(|(_, tabs)| tabs).collect())
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
/// first, each with its row and the thread it opens in. A read keeps them:
/// the window calls [`forget_queued_tabs`] once it has kept them in its
/// stored tabs. A request with no device has none.
#[expect(
    clippy::disallowed_methods,
    reason = "off the bus: the tabs an agent opened for a device that was away"
)]
pub async fn list_queued_tabs(req: &RpcRequest<'_>) -> Result<Vec<QueuedTab>, IpcError> {
    let Some(caller) = req.caller else {
        return Ok(Vec::new());
    };
    Ok(req
        .tab_queue_store
        .take(&caller.device)
        .await?
        .into_iter()
        .map(|row| QueuedTab {
            id: QueuedTabId(row.id),
            thread_id: row.thread,
            uri: row.uri,
        })
        .collect())
}

/// The caller device's window kept these queued tabs: their rows go.
#[expect(
    clippy::disallowed_methods,
    reason = "off the bus: the tabs an agent opened for a device that was away"
)]
pub async fn forget_queued_tabs(
    req: &RpcRequest<'_>,
    ids: Vec<QueuedTabId>,
) -> Result<(), IpcError> {
    let device = &req.caller()?.device;
    let ids: Vec<i64> = ids.into_iter().map(|id| id.0).collect();
    req.tab_queue_store.forget(device, &ids).await?;
    Ok(())
}

/// A queued tab's row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(transparent)]
#[specta(transparent)]
pub struct QueuedTabId(#[specta(type = oxplow_domain::JsNumber)] pub i64);

/// A tab waiting for a device: its row, the page, and the thread it opens in.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct QueuedTab {
    pub id: QueuedTabId,
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
        // The tablet has stored tabs here, and is away. A device nobody used
        // is refused.
        let stranger = oxplow_domain::DeviceId::generate();
        let refused = svc
            .commands
            .run(
                &oxplow_domain::Actor::Agent {
                    session_id: None,
                    thread_id: Some(thread),
                    stream_id: None,
                },
                "oxplow.page.show",
                json!({ "spec": "title: Waiting\nview: { markdown: hi }\n", "client": stranger.to_string() }),
                false,
            )
            .await
            .unwrap_err();
        assert!(
            refused.to_string().contains("not known")
                || refused.to_string().contains("known to this project"),
            "{refused}"
        );
        crate::dispatch_as(
            "save_tab_session",
            json!({ "threadId": thread.to_string(), "sessionJson": "{}", "revision": null }),
            &svc,
            Some(&tablet),
        )
        .await
        .unwrap();
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
                crate::dispatch_as("list_queued_tabs", json!({}), &svc, caller.as_ref())
                    .await
                    .unwrap()
            }
        };
        assert_eq!(take(Some(&mac)).await, json!([]));
        assert_eq!(take(None).await, json!([]));
        let waiting = take(Some(&tablet)).await;
        let id = waiting[0]["id"].clone();
        assert_eq!(
            waiting,
            json!([{ "id": id, "threadId": thread.to_string(), "uri": uri }])
        );
        assert_eq!(take(Some(&tablet)).await, waiting, "read, not yet kept");
        let forget = |caller: Option<&oxplow_domain::Caller>| {
            let svc = svc.clone();
            let caller = caller.cloned();
            let id = id.clone();
            async move {
                crate::dispatch_as(
                    "forget_queued_tabs",
                    json!({ "ids": [id] }),
                    &svc,
                    caller.as_ref(),
                )
                .await
            }
        };
        forget(Some(&mac)).await.unwrap();
        assert_eq!(take(Some(&tablet)).await, waiting, "only its own rows go");
        assert_eq!(forget(None).await.unwrap_err().code, "INVALID");
        forget(Some(&tablet)).await.unwrap();
        assert_eq!(take(Some(&tablet)).await, json!([]), "kept, then gone");
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

    /// Continue from another device: the other devices' tabs of a thread,
    /// never the caller's own, and none of another thread.
    #[tokio::test]
    async fn another_devices_tabs_of_a_thread_are_read_for_continue() {
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
        let ask = |name: &'static str, args: serde_json::Value, caller: &oxplow_domain::Caller| {
            let svc = svc.clone();
            let caller = caller.clone();
            async move { crate::dispatch_as(name, args, &svc, Some(&caller)).await }
        };
        let session = |thread: &str, json: &str| json!({ "threadId": thread, "sessionJson": json, "revision": null });
        ask(
            "save_tab_session",
            session(&thread, "{\"tabs\":[\"mac\"]}"),
            &mac,
        )
        .await
        .unwrap();
        ask(
            "save_tab_session",
            session(&thread, "{\"tabs\":[\"tablet\"]}"),
            &tablet,
        )
        .await
        .unwrap();
        let other = ask(
            "list_other_device_tabs",
            json!({ "threadId": thread }),
            &tablet,
        )
        .await
        .unwrap();
        assert_eq!(
            other,
            json!([{ "device": mac.device.as_str(), "name": null, "sessionJson": "{\"tabs\":[\"mac\"]}" }])
        );
        assert_eq!(
            ask(
                "list_other_device_tabs",
                json!({ "threadId": "thr999" }),
                &tablet
            )
            .await
            .unwrap(),
            json!([])
        );
    }

    /// A window's report reads back as what it shows, as its page load and
    /// device; a malformed thread, or a report with no caller, is refused.
    #[tokio::test]
    async fn a_windows_report_is_read_back() {
        let (svc, _dir) = crate::test_support::services().await;
        let caller = crate::test_support::caller();
        let report = || {
            json!({ "report": {
                "focused": true, "visible": true, "shown_thread": "thr1",
                "detail_json": null
            } })
        };
        crate::dispatch_as("report_tabs", report(), &svc, Some(&caller))
            .await
            .unwrap();
        let t = oxplow_domain::ThreadId::try_from_str("thr1").unwrap();
        let windows = svc.client_reports.windows(oxplow_domain::Timestamp::now());
        assert_eq!(windows[0].shown_thread, Some(t));
        assert_eq!(windows[0].connection, caller.connection.as_str());
        assert_eq!(windows[0].device, caller.device.as_str());

        let anonymous = crate::dispatch("report_tabs", report(), &svc)
            .await
            .unwrap_err();
        assert_eq!(anonymous.code, "INVALID");

        let err = crate::dispatch_as(
            "report_tabs",
            json!({ "report": { "focused": true, "visible": true, "shown_thread": "nope",
                                "detail_json": null } }),
            &svc,
            Some(&caller),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID");
    }
}
