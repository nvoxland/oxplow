//! Cores for the `open_page` command module — the UI reports which page
//! the human has open in a thread, so an agent can "look at what I'm
//! looking at" (MCP `get_open_page`). Ephemeral, held in the thread
//! runtime registry.

use oxplow_app::thread_runtime::OpenPage;
use oxplow_app::Services;
use oxplow_domain::{ThreadId, Timestamp};

use crate::error::IpcError;

/// Record the page open in `thread_id` (`page_id` absent = nothing open).
/// `detail_json` carries page-specific context (a lens's id + params).
pub async fn report_open_page(
    svc: &Services,
    thread_id: String,
    page_id: Option<String>,
    kind: Option<String>,
    detail_json: Option<String>,
) -> Result<(), IpcError> {
    let thread = ThreadId::try_from_str(&thread_id).ok_or_else(|| {
        oxplow_domain::DomainError::Invalid(format!("{thread_id:?} is not a thread id"))
    })?;
    let page = page_id.map(|page_id| OpenPage {
        kind: kind.unwrap_or_else(|| page_id.split(':').next().unwrap_or_default().to_string()),
        page_id,
        detail_json,
        reported_at: Timestamp::now(),
    });
    svc.thread_runtime.set_open_page(&thread, page);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn reports_and_clears_the_open_page() {
        let (svc, _dir) = crate::test_support::services();
        crate::dispatch(
            "report_open_page",
            json!({ "threadId": "thr1", "pageId": "lens:review/waiting", "kind": "lens", "detailJson": "{\"params\":{}}" }),
            &svc,
        )
        .await
        .unwrap();
        let t = ThreadId::try_from_str("thr1").unwrap();
        let page = svc.thread_runtime.open_page(&t).unwrap();
        assert_eq!(page.page_id, "lens:review/waiting");
        assert_eq!(page.kind, "lens");

        crate::dispatch("report_open_page", json!({ "threadId": "thr1" }), &svc)
            .await
            .unwrap();
        assert_eq!(svc.thread_runtime.open_page(&t), None);

        let err = crate::dispatch(
            "report_open_page",
            json!({ "threadId": "nope", "pageId": "x" }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID");
    }
}
