//! Cores for the `semantic` command module — read access to the
//! semantic layer (`v_*` views). See `.context/semantic-layer.md`.

use oxplow_app::Services;
use oxplow_db::{SqlCell, SqlQueryResult};

use crate::error::IpcError;

/// Run one read-only `SELECT`/`WITH` over the semantic layer, with
/// positional `params` (`?1`, `?2`, …) and a row cap. `raw` is the
/// person's explorer reading physical tables too — this IPC read only;
/// the MCP tool has no such switch (P4.3).
pub async fn query_sql(
    svc: &Services,
    sql: String,
    params: Option<Vec<SqlCell>>,
    limit: Option<u32>,
    raw: Option<bool>,
) -> Result<SqlQueryResult, IpcError> {
    Ok(svc
        .sql
        .run(
            oxplow_db::SqlQuery::new(sql)
                .positional(params.unwrap_or_default())
                .limit(limit.map(|l| l as usize))
                .raw(raw.unwrap_or(false)),
        )
        .await?)
}

/// The person's left-nav layout (P6.G1): each panel's order, and whether
/// it's hidden or collapsed. UI-only.
pub async fn get_panel_layout(svc: &Services) -> Result<Vec<oxplow_db::PanelPlacement>, IpcError> {
    Ok(svc.panel_layout_store.get().await?)
}

/// Everything the person's oxplow remembers outside any one tab: page
/// memory and app preferences (`ui_preference`). UI-only; an agent reads
/// `v_ui_preference`.
pub async fn get_ui_preferences(svc: &Services) -> Result<Vec<oxplow_db::UiPreference>, IpcError> {
    Ok(svc.ui_preference_store.all().await?)
}

/// Remember one value (JSON text), or forget it with none.
#[expect(
    clippy::disallowed_methods,
    reason = "off the bus: page memory and app preferences"
)]
pub async fn set_ui_preference(
    svc: &Services,
    scope: String,
    key: String,
    value_json: Option<String>,
) -> Result<(), IpcError> {
    Ok(svc.ui_preference_store.set(scope, key, value_json).await?)
}

/// Replace the layout (the order is the list's).
#[expect(
    clippy::disallowed_methods,
    reason = "off the bus: the left-nav panel layout"
)]
pub async fn set_panel_layout(
    svc: &Services,
    layout: Vec<oxplow_db::PanelPlacement>,
) -> Result<(), IpcError> {
    Ok(svc.panel_layout_store.set(layout).await?)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[tokio::test]
    async fn query_sql_dispatches_and_reads_views() {
        let (svc, _dir) = crate::test_support::services().await;
        let out = crate::dispatch(
            "query_sql",
            json!({ "sql": "SELECT count(*) AS n FROM v_stream WHERE kind = ?1", "params": ["primary"], "limit": 5 }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(out["columns"], json!(["n"]));
        assert_eq!(out["truncated"], json!(false));
        assert!(out["rows"][0][0].is_number());
    }

    #[tokio::test]
    async fn query_sql_rejects_writes_as_invalid() {
        let (svc, _dir) = crate::test_support::services().await;
        let err = crate::dispatch("query_sql", json!({ "sql": "DELETE FROM task" }), &svc)
            .await
            .unwrap_err();
        assert_eq!(err.code, "INVALID");
    }

    /// P4.3 (tsk488): a physical table is refused unless the person's
    /// explorer asks for a raw read.
    #[tokio::test]
    async fn query_sql_reads_tables_only_when_raw() {
        let (svc, _dir) = crate::test_support::services().await;
        let err = crate::dispatch(
            "query_sql",
            json!({ "sql": "SELECT count(*) FROM work_item" }),
            &svc,
        )
        .await
        .unwrap_err();
        assert!(err.message.contains("v_work_item"), "{}", err.message);
        let out = crate::dispatch(
            "query_sql",
            json!({ "sql": "SELECT count(*) FROM work_item", "raw": true }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(out["reads"]["tables"], json!(["work_item"]));
    }
}
