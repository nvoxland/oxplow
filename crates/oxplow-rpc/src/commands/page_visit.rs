//! Cores for the `page_visit` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use oxplow_app::Services;
use oxplow_db::analytics_stores::PageVisitStore as _;
use oxplow_db::PageVisit;
use serde::{Deserialize, Serialize};
use specta::Type;

use crate::error::IpcError;

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct VisitedPage {
    pub page_kind: String,
    pub page_id: String,
    #[specta(type = oxplow_domain::JsNumber)]
    pub visit_count: i64,
}

pub async fn record_page_visit(
    svc: &Services,
    page_kind: String,
    page_id: String,
    label: Option<String>,
    thread_id: Option<String>,
) -> Result<PageVisit, IpcError> {
    #[expect(clippy::disallowed_methods, reason = "off the bus: page visits")]
    let visit = svc
        .page_visit_store
        .record(&page_kind, &page_id, label.as_deref(), thread_id.as_deref())
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

pub async fn list_recent_page_visits(
    svc: &Services,
    limit: u32,
    thread_id: Option<String>,
) -> Result<Vec<PageVisit>, IpcError> {
    Ok(svc
        .page_visit_store
        .list_recent(limit as usize, thread_id.as_deref())
        .await?)
}

pub async fn top_visited_pages(
    svc: &Services,
    limit: u32,
    thread_id: Option<String>,
) -> Result<Vec<VisitedPage>, IpcError> {
    let pairs = svc
        .page_visit_store
        .list_top(limit as usize, thread_id.as_deref())
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
