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

/// Settings → Data: every published model, and the entities extensions
/// declare that haven't synced. No row counts: the UI counts each model
/// with `query_sql`, so one too big to count costs only its own row.
/// UI-only: an agent reads `v_model` and counts with `query_sql`.
pub async fn list_data_entities(
    svc: &Services,
) -> Result<Vec<oxplow_app::semantic_catalog::DataEntity>, IpcError> {
    // Extension-declared entities come from the primary worktree (their
    // data is project-global).
    let root = svc.worktrees.resolve(None).await;
    Ok(
        oxplow_app::semantic_catalog::data_entities(&svc.sql, &svc.extension_catalog, &root)
            .await?,
    )
}

/// What the person can ask (the catalog page, contextual prompts): every
/// capability's questions and the stream's enabled extensions' prompts.
/// UI-only: the agent is who gets asked.
pub async fn prompt_catalog(
    svc: &Services,
    stream_id: Option<String>,
) -> Result<Vec<oxplow_app::prompt_catalog::CatalogPrompt>, IpcError> {
    let root = svc.worktrees.resolve(stream_id.as_deref()).await;
    Ok(oxplow_app::prompt_catalog::prompt_catalog(
        svc.extension_catalog.get(&root).as_ref(),
    ))
}

/// Every setting with its value and where it comes from (P6.H1): the
/// Settings view. UI-only: an agent reads `config.list_keys`.
pub async fn effective_config(
    svc: &Services,
) -> Result<Vec<oxplow_app::effective_config::EffectiveSetting>, IpcError> {
    let config = svc
        .config
        .read()
        .map_err(|_| IpcError::internal("config lock poisoned"))?
        .clone();
    let root = svc.worktrees.resolve(None).await;
    let extensions = svc.extension_catalog.get(&root);
    let ai = svc.ai.settings().ok();
    let global = oxplow_config::global_config_dir();
    Ok(oxplow_app::effective_config::effective_config(
        &config,
        &svc.layout.project_dir,
        ai.as_ref(),
        global.as_deref(),
        extensions.as_ref(),
    ))
}

/// The person's left-nav layout (P6.G1): each panel's order, and whether
/// it's hidden or collapsed. UI-only.
pub async fn get_panel_layout(svc: &Services) -> Result<Vec<oxplow_db::PanelPlacement>, IpcError> {
    Ok(svc.panel_layout_store.get().await?)
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
    #[tokio::test]
    async fn prompt_catalog_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch("prompt_catalog", serde_json::json!({}), &svc)
            .await
            .unwrap();
        assert!(out
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["about"] == "effort" && p["source"]["kind"] == "capability"));
    }

    #[tokio::test]
    async fn list_data_entities_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch("list_data_entities", serde_json::json!({}), &svc)
            .await
            .unwrap();
        let task = out
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["name"] == "v_task")
            .unwrap();
        assert_eq!(task["kind"], "sql");
        // tsk1065: the listing counts nothing: a model too big to count in
        // the query timeout failed the whole list. The UI counts per row.
        assert!(task.get("rows").is_none(), "{task}");
    }

    use serde_json::json;

    #[tokio::test]
    async fn query_sql_dispatches_and_reads_views() {
        let (svc, _dir) = crate::test_support::services();
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
        let (svc, _dir) = crate::test_support::services();
        let err = crate::dispatch("query_sql", json!({ "sql": "DELETE FROM task" }), &svc)
            .await
            .unwrap_err();
        assert_eq!(err.code, "INVALID");
    }

    /// P4.3 (tsk488): a physical table is refused unless the person's
    /// explorer asks for a raw read.
    #[tokio::test]
    async fn query_sql_reads_tables_only_when_raw() {
        let (svc, _dir) = crate::test_support::services();
        let err = crate::dispatch(
            "query_sql",
            json!({ "sql": "SELECT count(*) FROM task" }),
            &svc,
        )
        .await
        .unwrap_err();
        assert!(err.message.contains("v_task"), "{}", err.message);
        let out = crate::dispatch(
            "query_sql",
            json!({ "sql": "SELECT count(*) FROM task", "raw": true }),
            &svc,
        )
        .await
        .unwrap();
        assert_eq!(out["reads"]["tables"], json!(["task"]));
    }
}
