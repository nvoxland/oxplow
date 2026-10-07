//! Metric commands for the desktop (P4.7). Metric *reads* are SQL — the
//! grid (`metric_grid()`), `v_metric_spec` and `v_metric_catalog` through
//! `query_sql`; what's left here is the person's switch, which runs the
//! `oxplow.metric.enable` bus command like every other write.

use oxplow_app::Services;
use oxplow_domain::Actor;

use crate::error::IpcError;

/// Turn metrics on or off in this project — the Catalog toggle and its
/// per-section "enable all" — as the person, through `oxplow.metric.enable`.
pub async fn enable_metrics(
    svc: &Services,
    keys: Vec<String>,
    enabled: bool,
) -> Result<(), IpcError> {
    svc.commands
        .run(
            &Actor::Human,
            oxplow_app::commands::metric::ENABLE,
            serde_json::json!({ "keys": keys, "enabled": enabled }),
            false,
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    /// P4.7 (tsk492): the catalog toggle goes through the command — the
    /// catalog reads it back through SQL.
    #[tokio::test]
    async fn enabling_a_metric_shows_in_the_catalog() {
        let (svc, _dir) = crate::test_support::services();
        svc.metrics.seed_catalog().await;
        let enabled = |svc: &crate::RpcContext| {
            let svc = svc.clone();
            async move {
                crate::dispatch(
                    "query_sql",
                    json!({ "sql": "SELECT enabled FROM v_metric_catalog WHERE key = 'oxplow.rust.unsafe_blocks'" }),
                    &svc,
                )
                .await
                .unwrap()["rows"][0][0]
                    .clone()
            }
        };
        assert_eq!(enabled(&svc).await, json!(0));
        crate::dispatch(
            "enable_metrics",
            json!({ "keys": ["oxplow.rust.unsafe_blocks"], "enabled": true }),
            &svc,
        )
        .await
        .unwrap();
        svc.metrics.seed_catalog().await;
        assert_eq!(enabled(&svc).await, json!(1));
        let err = crate::dispatch(
            "enable_metrics",
            json!({ "keys": ["nope"], "enabled": true }),
            &svc,
        )
        .await
        .unwrap_err();
        assert!(err.message.contains("no metric `nope`"), "{}", err.message);
    }
}
