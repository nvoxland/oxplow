//! `sql.read/query { sql, params?, limit? }`: one read-only statement over
//! the published models (`v_*`), through the semantic layer's gateway
//! (the limit and the timeout are its).

use std::sync::Arc;

use oxplow_domain::{CommandError, DomainError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::super::ops::Op;
use super::super::util::{failed, parse, schema, ServicesSlot};
use super::super::{Handler, HandlerOutput, Invocation};

/// The command its operation is declared as.
pub const SQL_QUERY: &str = "oxplow.sql.query";

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QuerySqlInput {
    /// One read-only `SELECT` or `WITH` statement over the published
    /// models (`v_model` lists them, `v_model_column` documents their
    /// columns). Use `?1`, `?2`, … for parameters.
    pub sql: String,
    /// Positional parameter values for `?1`, `?2`, ….
    #[serde(default)]
    pub params: Option<Vec<serde_json::Value>>,
    /// Row cap (default 500, max 10000). `truncated: true` in the result
    /// means more rows existed.
    #[serde(default)]
    pub limit: Option<u32>,
}

pub fn ops(services: ServicesSlot) -> Vec<Op> {
    vec![Op::new(
        "sql.read",
        "query",
        schema::<QuerySqlInput>(),
        false,
        Handler::External(Arc::new(move |_: Invocation, input| {
            let services = services.clone();
            Box::pin(async move {
                let QuerySqlInput { sql, params, limit } = parse(input)?;
                let svc = services.get()?;
                let out = svc
                    .sql
                    .clone()
                    .query_sql(
                        &sql,
                        params
                            .unwrap_or_default()
                            .into_iter()
                            .map(Into::into)
                            .collect(),
                        limit.map(|l| l as usize),
                    )
                    .await
                    .map_err(|e| match e {
                        DomainError::Invalid(message) => CommandError::Invalid {
                            field: Some("/sql".into()),
                            message,
                        },
                        other => failed(other),
                    })?;
                Ok(HandlerOutput {
                    result: serde_json::to_value(out).map_err(failed)?,
                    ..HandlerOutput::default()
                })
            })
        })),
    )
    .returns::<oxplow_db::SqlQueryResult>()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::Actor;
    use serde_json::json;

    fn anonymous() -> Actor {
        Actor::Agent {
            session_id: None,
            thread_id: None,
            stream_id: None,
        }
    }

    /// A model reads through the bus with bound params and a row cap,
    /// leaving no record; a write or a physical table is refused as
    /// invalid.
    #[tokio::test]
    async fn a_model_queries_through_the_bus() {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let svc = crate::Services::in_memory(dir.path()).await.unwrap();
        let out = svc
            .commands
            .run(
                &anonymous(),
                SQL_QUERY,
                json!({ "sql": "SELECT view FROM v_model WHERE view = ?1", "params": ["v_work_item"], "limit": 5 }),
                false,
            )
            .await
            .unwrap();
        assert_eq!(out.result["columns"], json!(["view"]));
        assert_eq!(out.result["rows"], json!([["v_work_item"]]));
        assert!(out.audit_id.is_none(), "a read leaves no record");

        let err = svc
            .commands
            .run(
                &anonymous(),
                SQL_QUERY,
                json!({ "sql": "DELETE FROM task" }),
                false,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, CommandError::Invalid { .. }), "{err:?}");
    }
}
