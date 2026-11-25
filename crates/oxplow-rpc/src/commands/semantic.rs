//! Cores for the `semantic` command module — read access to the
//! semantic layer (`v_*` views). See `.context/semantic-layer.md`.

use oxplow_app::Services;
use oxplow_db::{SqlCell, SqlQueryResult};

use crate::error::IpcError;
use crate::RpcRequest;

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

/// Everything the caller's device remembers outside any one tab (page
/// memory and app preferences, `ui_preference`) and every component's
/// memory (`component_memory`). A request with no device gets the
/// components' alone. UI-only; an agent reads `v_ui_preference` and
/// `v_component_memory`.
pub async fn get_ui_preferences(
    req: &RpcRequest<'_>,
) -> Result<Vec<oxplow_db::UiPreference>, IpcError> {
    Ok(req
        .ui_preference_store
        .all(req.caller.map(|c| &c.device))
        .await?)
}

/// Remember one value (JSON text), or forget it with none: the caller
/// device's own, or a component's, which every device shares.
#[expect(
    clippy::disallowed_methods,
    reason = "off the bus: page memory and app preferences"
)]
pub async fn set_ui_preference(
    req: &RpcRequest<'_>,
    scope: String,
    key: String,
    value_json: Option<String>,
) -> Result<(), IpcError> {
    Ok(req
        .ui_preference_store
        .set(req.caller.map(|c| &c.device), scope, key, value_json)
        .await?)
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

    /// Two devices keep their own app preference and both read one
    /// component's memory; with no device a component's memory is written and
    /// read, an app preference is refused.
    #[tokio::test]
    async fn preferences_are_each_devices_but_components_are_shared() {
        let (svc, _dir) = crate::test_support::services().await;
        let mac = crate::test_support::caller();
        let tablet = oxplow_domain::Caller {
            device: oxplow_domain::DeviceId::generate(),
            connection: oxplow_domain::ConnectionId::generate(),
        };
        let set = |caller: Option<&oxplow_domain::Caller>, scope: &str, value: &str| {
            let args = json!({ "scope": scope, "key": "k", "valueJson": value });
            let caller = caller.cloned();
            let svc = svc.clone();
            async move { crate::dispatch_as("set_ui_preference", args, &svc, caller.as_ref()).await }
        };
        let get = |caller: Option<&oxplow_domain::Caller>| {
            let caller = caller.cloned();
            let svc = svc.clone();
            async move {
                crate::dispatch_as("get_ui_preferences", json!({}), &svc, caller.as_ref())
                    .await
                    .unwrap()
            }
        };
        set(Some(&mac), "app", "1").await.unwrap();
        set(Some(&tablet), "app", "2").await.unwrap();
        set(Some(&tablet), "component:acme/board", "3")
            .await
            .unwrap();
        assert_eq!(
            get(Some(&mac)).await,
            json!([
                { "scope": "app", "key": "k", "valueJson": "1" },
                { "scope": "component:acme/board", "key": "k", "valueJson": "3" }
            ])
        );
        assert_eq!(
            get(Some(&tablet)).await,
            json!([
                { "scope": "app", "key": "k", "valueJson": "2" },
                { "scope": "component:acme/board", "key": "k", "valueJson": "3" }
            ])
        );
        assert_eq!(set(None, "app", "4").await.unwrap_err().code, "INVALID");
        set(None, "component:acme/board", "5").await.unwrap();
        assert_eq!(
            get(None).await,
            json!([{ "scope": "component:acme/board", "key": "k", "valueJson": "5" }])
        );
    }

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
