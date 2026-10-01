//! The capability providers the UI reads (`v_capability_provider`, P6b):
//! core's, published at boot. External providers publish theirs while
//! they run (`providers::registry`). Feature flags always come from the
//! provider itself — never from a manifest.
//!
//! Which provider is a capability's **active** one (P7.A2) is the
//! project's `activeProviders` (a person's key): [`is_active`] is the one
//! rule every published row's `active` follows, and [`apply_active`]
//! restates it — the work-items registry's default for `create`, and the
//! rows — when the config changes.

use oxplow_db::{CapabilityProvider, SqliteCapabilityStore};
use oxplow_domain::DomainError;
use serde_json::{json, Value};

/// Restate the table as core's providers: oxplow's work items, the VCS
/// and the knowledge provider. A previous run's external rows go — their
/// instances publish again when the provider registry starts them.
pub async fn publish_core(svc: &crate::Services) -> Result<(), DomainError> {
    let config = crate::config_service::read_config(&svc.config);
    let row = |capability: &str, provider: &str, features: Value| CapabilityProvider {
        capability: capability.into(),
        provider: provider.into(),
        extension: None,
        features,
        active: is_active(&config, capability, provider),
    };
    // Oxplow's own work items always exist: a missing provider is a boot
    // bug, not a capability to leave out.
    let work_items = svc
        .work_items
        .get(crate::work_items::PROVIDER)
        .map_err(|e| DomainError::Invariant(format!("oxplow's work items provider: {e}")))?;
    let mut rows = vec![row(
        "work_items",
        &work_items.id,
        serde_json::to_value(work_items.features).unwrap_or(Value::Null),
    )];
    rows.push(row(
        "vcs",
        svc.vcs.rev_kind(),
        serde_json::to_value(svc.vcs.features()).unwrap_or(Value::Null),
    ));
    rows.push(row("knowledge", svc.knowledge.provider(), json!({})));
    SqliteCapabilityStore::new(svc.db.clone()).reset(rows).await
}

/// `capability`'s active provider: the one `activeProviders` names, else
/// oxplow's own.
pub fn active_provider(config: &oxplow_config::OxplowConfig, capability: &str) -> String {
    config
        .active_providers
        .get(capability)
        .cloned()
        .unwrap_or_else(|| oxplow_domain::work_items::OXPLOW.to_string())
}

/// Whether `provider` is `capability`'s active provider. A capability a
/// project can't swap (`vcs`, `knowledge`) has only core's, always
/// active.
pub fn is_active(config: &oxplow_config::OxplowConfig, capability: &str, provider: &str) -> bool {
    !oxplow_config::SWAPPABLE_CAPABILITIES.contains(&capability)
        || active_provider(config, capability) == provider
}

/// Restate the active providers from `config`: the work-items registry's
/// default for a `create` without a provider, and each swappable
/// capability's `active` column.
pub async fn apply_active(
    config: &oxplow_config::OxplowConfig,
    work_items: &oxplow_domain::work_items::WorkItemsRegistry,
    db: &oxplow_db::Database,
) -> Result<(), DomainError> {
    work_items.set_active(&active_provider(config, "work_items"));
    let store = SqliteCapabilityStore::new(db.clone());
    for capability in oxplow_config::SWAPPABLE_CAPABILITIES {
        store
            .set_active(capability, &active_provider(config, capability))
            .await?;
    }
    Ok(())
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

    /// P7.A2: the `active` column follows `activeProviders` — oxplow's own
    /// by default, the named provider once set — for core's rows and an
    /// external one's alike; a capability nobody swaps stays active.
    #[tokio::test]
    async fn the_active_column_follows_the_config() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let store = SqliteCapabilityStore::new(fx.svc.db.clone());
        publish_core(&fx.svc).await.unwrap();
        store
            .upsert(CapabilityProvider {
                capability: "work_items".into(),
                provider: "linear".into(),
                extension: Some("tracker".into()),
                features: json!({}),
                active: false,
            })
            .await
            .unwrap();
        let active = |rows: Vec<CapabilityProvider>| -> Vec<(String, String)> {
            rows.into_iter()
                .filter(|r| r.active)
                .map(|r| (r.capability, r.provider))
                .collect()
        };
        let pair = |c: &str, p: &str| (c.to_string(), p.to_string());
        assert_eq!(
            active(store.list().await.unwrap()),
            vec![
                pair("knowledge", "oxplow"),
                pair("vcs", "git"),
                pair("work_items", "oxplow")
            ]
        );
        fx.svc
            .config
            .write()
            .unwrap()
            .active_providers
            .insert("work_items".into(), "linear".into());
        let config = crate::config_service::read_config(&fx.svc.config);
        apply_active(&config, &fx.svc.work_items, &fx.svc.db)
            .await
            .unwrap();
        assert_eq!(
            active(store.list().await.unwrap()),
            vec![
                pair("knowledge", "oxplow"),
                pair("vcs", "git"),
                pair("work_items", "linear")
            ]
        );
        assert_eq!(fx.svc.work_items.active(), "linear");
        assert!(is_active(&config, "vcs", "git"));
        assert!(!is_active(&config, "work_items", "oxplow"));
    }
}
