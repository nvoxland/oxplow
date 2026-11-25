//! Cores for the `tabs` command module — a window reports the tabs it
//! shows (per thread, which is selected, the thread it shows, its focus
//! and visibility, and its active page's detail), so an agent can see what
//! the person sees (`oxplow_list_tabs`, `oxplow_get_open_page`). Held in
//! memory (`oxplow_app::client_reports`).

use oxplow_app::client_reports::{TabReport, TabReportAnswer};
use oxplow_domain::Timestamp;

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

#[cfg(test)]
mod tests {
    use serde_json::json;

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
