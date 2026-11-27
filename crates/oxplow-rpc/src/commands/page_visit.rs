//! Cores for the `page_visit` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use oxplow_app::Services;
use oxplow_db::analytics_stores::PageVisitStore as _;
use oxplow_db::PageVisit;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::IpcError;
use crate::RpcRequest;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct VisitedPage {
    pub page_kind: String,
    pub page_id: String,
    #[specta(type = oxplow_domain::JsNumber)]
    pub visit_count: i64,
}

/// A visit belongs to the device that opened the page.
pub async fn record_page_visit(
    req: &RpcRequest<'_>,
    page_kind: String,
    page_id: String,
    label: Option<String>,
    thread_id: Option<String>,
) -> Result<PageVisit, IpcError> {
    #[expect(clippy::disallowed_methods, reason = "off the bus: page visits")]
    let visit = req
        .page_visit_store
        .record(
            &page_kind,
            &page_id,
            label.as_deref(),
            thread_id.as_deref(),
            req.caller.map(|c| &c.device),
        )
        .await?;
    Ok(visit)
}

/// The visit `record_page_visit` returned ended, after `duration_ms` shown.
pub async fn finish_page_visit(
    svc: &Services,
    id: String,
    duration_ms: u32,
) -> Result<(), IpcError> {
    #[expect(clippy::disallowed_methods, reason = "off the bus: page visits")]
    svc.page_visit_store
        .finish(&id, i64::from(duration_ms))
        .await?;
    Ok(())
}

/// The caller's device's visits and the ones no device owns; a request with
/// no device reads every device's.
pub async fn list_recent_page_visits(
    req: &RpcRequest<'_>,
    limit: u32,
    thread_id: Option<String>,
) -> Result<Vec<PageVisit>, IpcError> {
    Ok(req
        .page_visit_store
        .list_recent(
            limit as usize,
            thread_id.as_deref(),
            req.caller.map(|c| &c.device),
        )
        .await?)
}

pub async fn top_visited_pages(
    req: &RpcRequest<'_>,
    limit: u32,
    thread_id: Option<String>,
) -> Result<Vec<VisitedPage>, IpcError> {
    let pairs = req
        .page_visit_store
        .list_top(
            limit as usize,
            thread_id.as_deref(),
            req.caller.map(|c| &c.device),
        )
        .await?;
    Ok(pairs
        .into_iter()
        .map(|(page_kind, page_id, visit_count)| VisitedPage {
            page_kind,
            page_id,
            visit_count,
        })
        .collect())
}

#[expect(clippy::disallowed_methods, reason = "off the bus: forgetting a page")]
pub async fn forget_page(
    svc: &Services,
    page_kind: String,
    page_id: String,
) -> Result<(), IpcError> {
    svc.page_visit_store
        .forget_page(&page_kind, &page_id)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn a_recorded_visit_is_finished_with_how_long_it_was_shown() {
        let (svc, _dir) = crate::test_support::services().await;
        let visit = crate::dispatch(
            "record_page_visit",
            serde_json::json!({"pageKind": "alerts", "pageId": "oxplow:oxplow/page?page=alerts", "label": "Alerts", "threadId": null}),
            &svc,
        )
        .await
        .unwrap();
        crate::dispatch(
            "finish_page_visit",
            serde_json::json!({"id": visit["id"], "durationMs": 1500}),
            &svc,
        )
        .await
        .unwrap();
        let recent = crate::dispatch(
            "list_recent_page_visits",
            serde_json::json!({"limit": 10, "threadId": null}),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(recent[0]["duration_ms"], 1500, "{recent}");
    }

    /// Each device lists the pages it opened and the ones from before
    /// devices; a request with no device lists every device's.
    #[tokio::test]
    async fn each_device_lists_its_own_page_visits() {
        let (svc, _dir) = crate::test_support::services().await;
        let mac = crate::test_support::caller();
        let tablet = oxplow_domain::Caller {
            device: oxplow_domain::DeviceId::generate(),
            connection: oxplow_domain::ConnectionId::generate(),
        };
        let visit = |page: &str| serde_json::json!({"pageKind": "task", "pageId": page, "label": null, "threadId": null});
        for (caller, page) in [
            (Some(&mac), "on-mac"),
            (Some(&tablet), "on-tablet"),
            (None, "old"),
        ] {
            crate::dispatch_as("record_page_visit", visit(page), &svc, caller)
                .await
                .unwrap();
        }
        let list = |caller: Option<&oxplow_domain::Caller>| {
            let svc = svc.clone();
            let caller = caller.cloned();
            async move {
                let out = crate::dispatch_as(
                    "list_recent_page_visits",
                    serde_json::json!({"limit": 10, "threadId": null}),
                    &svc,
                    caller.as_ref(),
                )
                .await
                .unwrap();
                let mut pages: Vec<String> = out
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v["page_id"].as_str().unwrap().to_string())
                    .collect();
                pages.sort();
                pages
            }
        };
        assert_eq!(list(Some(&mac)).await, ["old", "on-mac"]);
        assert_eq!(list(Some(&tablet)).await, ["old", "on-tablet"]);
        assert_eq!(list(None).await, ["old", "on-mac", "on-tablet"]);
    }

    #[tokio::test]
    async fn list_recent_page_visits_dispatches() {
        let (svc, _dir) = crate::test_support::services().await;
        let out = crate::dispatch(
            "list_recent_page_visits",
            serde_json::json!({"limit": 10, "threadId": null}),
            &svc,
        )
        .await
        .unwrap();
        assert!(out.is_array());
    }
}
