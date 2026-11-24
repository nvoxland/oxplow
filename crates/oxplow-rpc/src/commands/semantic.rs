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

/// The viewer of `thread_id` (its own stream) or of `stream_id` alone.
async fn panel_viewer(
    svc: &Services,
    stream_id: Option<String>,
    thread_id: Option<String>,
) -> Result<oxplow_db::visibility_scope::Viewer, IpcError> {
    let thread = thread_id
        .as_deref()
        .and_then(oxplow_domain::ThreadId::try_from_str);
    let stream = stream_id
        .as_deref()
        .and_then(oxplow_domain::StreamId::try_from_str);
    Ok(svc
        .db
        .read(move |c| oxplow_db::panel_layout_store::viewer_tx(c, thread, stream))
        .await?)
}

/// The person's left-nav layout (P6.G1) as the viewer sees it: its
/// thread's list, its stream's and the project's, each panel's order and
/// whether it's collapsed (or, in the project's, hidden). UI-only.
pub async fn get_panel_layout(
    svc: &Services,
    stream_id: Option<String>,
    thread_id: Option<String>,
) -> Result<oxplow_db::ScopedPanelLayout, IpcError> {
    let viewer = panel_viewer(svc, stream_id, thread_id).await?;
    Ok(svc.panel_layout_store.get(viewer).await?)
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

/// Replace the lists the viewer sees (each in its order).
#[expect(
    clippy::disallowed_methods,
    reason = "off the bus: the left-nav panel layout"
)]
pub async fn set_panel_layout(
    svc: &Services,
    stream_id: Option<String>,
    thread_id: Option<String>,
    layout: oxplow_db::ScopedPanelLayout,
) -> Result<(), IpcError> {
    let viewer = panel_viewer(svc, stream_id, thread_id).await?;
    Ok(svc.panel_layout_store.set(viewer, layout).await?)
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
        let err = crate::dispatch("query_sql", json!({ "sql": "DELETE FROM threads" }), &svc)
            .await
            .unwrap_err();
        assert_eq!(err.code, "INVALID");
    }

    /// P4.3: a physical table is refused unless the person's
    /// explorer asks for a raw read.
    #[tokio::test]
    async fn query_sql_reads_tables_only_when_raw() {
        let (svc, _dir) = crate::test_support::services().await;
        let err = crate::dispatch(
            "query_sql",
            json!({ "sql": "SELECT count(*) FROM threads" }),
            &svc,
        )
        .await
        .unwrap_err();
        assert!(err.message.contains("v_thread"), "{}", err.message);
        let out = crate::dispatch(
            "query_sql",
            json!({ "sql": "SELECT count(*) FROM threads", "raw": true }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(out["reads"]["tables"], json!(["threads"]));
    }
}
