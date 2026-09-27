//! Cores for the `semantic` command module — read access to the
//! semantic layer (`v_*` views). See `.context/semantic-layer.md`.

use oxplow_app::Services;
use oxplow_db::{SchemaEntity, SemanticLayer, SqlCell, SqlQueryResult};

use crate::error::IpcError;

/// Run one read-only `SELECT`/`WITH` over the semantic layer, with
/// positional `params` (`?1`, `?2`, …) and a row cap.
pub async fn query_sql(
    svc: &Services,
    sql: String,
    params: Option<Vec<SqlCell>>,
    limit: Option<u32>,
) -> Result<SqlQueryResult, IpcError> {
    Ok(SemanticLayer::new(svc.db.clone())
        .query_sql(&sql, params.unwrap_or_default(), limit.map(|l| l as usize))
        .await?)
}

/// Every queryable entity with its column docs.
pub async fn describe_schema(svc: &Services) -> Result<Vec<SchemaEntity>, IpcError> {
    // Extension-declared entities come from the primary worktree (their
    // data is project-global).
    let root = svc.git.resolve_repo_dir(None).await;
    Ok(
        oxplow_app::semantic_catalog::describe_schema(&SemanticLayer::new(svc.db.clone()), &root)
            .await?,
    )
}

#[cfg(test)]
mod tests {
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

    #[tokio::test]
    async fn describe_schema_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch("describe_schema", json!({}), &svc)
            .await
            .unwrap();
        let names: Vec<&str> = out
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"v_task"));
        assert!(out[0]["columns"][0]["doc"].is_string());
    }
}
