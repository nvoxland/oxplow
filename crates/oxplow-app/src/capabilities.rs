//! The capability providers the UI reads (`v_capability_provider`, P6b):
//! core's, published at boot. External providers publish theirs while
//! they run (`providers::registry`). Feature flags always come from the
//! provider itself — never from a manifest.

use oxplow_db::{CapabilityProvider, SqliteCapabilityStore};
use oxplow_domain::DomainError;
use serde_json::{json, Value};

/// Restate the table as core's providers: oxplow's work items, the VCS
/// and the knowledge provider. A previous run's external rows go — their
/// instances publish again when the provider registry starts them.
pub async fn publish_core(svc: &crate::Services) -> Result<(), DomainError> {
    let row = |capability: &str, provider: &str, features: Value| CapabilityProvider {
        capability: capability.into(),
        provider: provider.into(),
        extension: None,
        features,
        active: true,
    };
    // Oxplow's own work items always exist: a missing provider is a boot
    // bug, not a capability to leave out.
    let work_items = svc
        .work_items
        .get(crate::work_items::PROVIDER)
        .map_err(|e| DomainError::Invariant(format!("oxplow's work items provider: {e}")))?;
    let mut rows = vec![row(
        "work_items",
        work_items.provider(),
        serde_json::to_value(work_items.features()).unwrap_or(Value::Null),
    )];
    rows.push(row(
        "vcs",
        svc.vcs.rev_kind(),
        serde_json::to_value(svc.vcs.features()).unwrap_or(Value::Null),
    ));
    rows.push(row("knowledge", svc.knowledge.provider(), json!({})));
    SqliteCapabilityStore::new(svc.db.clone()).reset(rows).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn core_providers_are_published_with_their_features() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let store = SqliteCapabilityStore::new(fx.svc.db.clone());
        store
            .upsert(CapabilityProvider {
                capability: "work_items".into(),
                provider: "stale".into(),
                extension: Some("gone".into()),
                features: json!({}),
                active: true,
            })
            .await
            .unwrap();
        publish_core(&fx.svc).await.unwrap();
        let rows = store.list().await.unwrap();
        let names: Vec<(&str, &str)> = rows
            .iter()
            .map(|r| (r.capability.as_str(), r.provider.as_str()))
            .collect();
        assert_eq!(
            names,
            vec![
                ("knowledge", "oxplow"),
                ("vcs", "git"),
                ("work_items", "oxplow")
            ],
            "core's, and a previous run's external row is gone"
        );
        let work_items = rows.iter().find(|r| r.capability == "work_items").unwrap();
        assert_eq!(work_items.features["in_progress_opens_effort"], true);
        assert_eq!(work_items.extension, None);
        // Readable as the model.
        let res = fx
            .svc
            .sql
            .query_sql(
                "SELECT provider, features FROM v_capability_provider WHERE capability = 'vcs'",
                vec![],
                None,
            )
            .await
            .unwrap();
        assert_eq!(res.rows.len(), 1);
    }
}
