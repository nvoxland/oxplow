//! Cores for the `tabs` command module — a window reports the tabs it
//! shows (per thread, which is selected, the thread it shows, its focus
//! and visibility, and its active page's detail), so an agent can see what
//! the person sees (`oxplow_list_tabs`, `oxplow_get_open_page`). Held in
//! memory (`oxplow_app::client_reports`).

use oxplow_app::client_reports::{TabReport, TabReportAnswer};
use oxplow_app::Services;
use oxplow_domain::Timestamp;

use crate::error::IpcError;

/// Take `report` as what its window shows now.
pub async fn report_tabs(svc: &Services, report: TabReport) -> Result<TabReportAnswer, IpcError> {
    let missing_runs = svc
        .page_runs
        .report_client_runs(&report.client, &report.runs);
    svc.client_reports.report(report, Timestamp::now());
    Ok(TabReportAnswer { missing_runs })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    /// A window's report reads back as its threads' tabs; a malformed
    /// thread is refused.
    #[tokio::test]
    async fn a_windows_report_is_read_back() {
        let (svc, _dir) = crate::test_support::services().await;
        crate::dispatch(
            "report_tabs",
            json!({ "report": {
                "client": "w1", "focused": true, "visible": true, "shown_thread": "thr1",
                "threads": [{ "thread": "thr1", "active": "oxplow:oxplow/wiki?slug=home",
                              "tabs": [{ "id": "oxplow:oxplow/wiki?slug=home", "kind": "wiki", "title": "Home" }] }],
                "detail_json": null
            } }),
            &svc,
        )
        .await
        .unwrap();
        let t = oxplow_domain::ThreadId::try_from_str("thr1").unwrap();
        let pages = svc
            .client_reports
            .open_pages(t, oxplow_domain::Timestamp::now());
        assert_eq!(pages[0].page_id, "oxplow:oxplow/wiki?slug=home");
        assert_eq!(pages[0].client, "w1");

        let err = crate::dispatch(
            "report_tabs",
            json!({ "report": { "client": "w1", "focused": true, "visible": true, "shown_thread": "nope",
                                "threads": [], "detail_json": null } }),
            &svc,
        )
        .await
        .unwrap_err();
        assert_eq!(err.code, "INVALID");
    }
}
