//! `capability_provider` (migration V128, P6b): which provider implements
//! which capability, with the feature flags the provider declares —
//! never a manifest's. Published as `v_capability_provider`, so the UI
//! hides what a provider can't do. Restated from what's running: the core
//! providers at boot, an external one while its instance runs.

use rusqlite::{params, Connection};
use serde_json::Value;

use oxplow_domain::DomainError;

use crate::database::{map_sql_err, Database};

/// One capability's provider and its features.
#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityProvider {
    /// `work_items`, `vcs`, `knowledge`.
    pub capability: String,
    /// The provider's name (`oxplow`, `git`, `fake`).
    pub provider: String,
    /// The extension it comes from; `None` for core's.
    pub extension: Option<String>,
    /// The capability's feature flags as the provider declares them.
    pub features: Value,
    /// Whether it's the capability's active provider (P7 chooses; every
    /// provider is active today).
    pub active: bool,
}

pub fn upsert_tx(conn: &Connection, row: &CapabilityProvider) -> Result<(), DomainError> {
    conn.execute(
        "INSERT INTO capability_provider (capability, provider, extension, features_json, active)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (capability, provider) DO UPDATE SET
           extension = excluded.extension, features_json = excluded.features_json,
           active = excluded.active",
        params![
            row.capability,
            row.provider,
            row.extension,
            row.features.to_string(),
            row.active
        ],
    )
    .map_err(map_sql_err)?;
    Ok(())
}

pub fn remove_tx(conn: &Connection, capability: &str, provider: &str) -> Result<(), DomainError> {
    conn.execute(
        "DELETE FROM capability_provider WHERE capability = ?1 AND provider = ?2",
        params![capability, provider],
    )
    .map_err(map_sql_err)?;
    Ok(())
}

pub fn list_tx(conn: &Connection) -> Result<Vec<CapabilityProvider>, DomainError> {
    let mut stmt = conn
        .prepare(
            "SELECT capability, provider, extension, features_json, active
               FROM capability_provider ORDER BY capability, provider",
        )
        .map_err(map_sql_err)?;
    let rows = stmt
        .query_map([], |r| {
            let features: String = r.get(3)?;
            Ok(CapabilityProvider {
                capability: r.get(0)?,
                provider: r.get(1)?,
                extension: r.get(2)?,
                features: serde_json::from_str(&features).unwrap_or(Value::Null),
                active: r.get(4)?,
            })
        })
        .map_err(map_sql_err)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sql_err)
}

#[derive(Clone)]
pub struct SqliteCapabilityStore {
    db: Database,
}

impl SqliteCapabilityStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn upsert(&self, row: CapabilityProvider) -> Result<(), DomainError> {
        self.db.transaction(move |tx| upsert_tx(tx, &row)).await
    }

    pub async fn remove(&self, capability: &str, provider: &str) -> Result<(), DomainError> {
        let (c, p) = (capability.to_string(), provider.to_string());
        self.db.transaction(move |tx| remove_tx(tx, &c, &p)).await
    }

    /// Replace every row with `rows`: what boot does, since a previous
    /// run's external providers aren't running now.
    pub async fn reset(&self, rows: Vec<CapabilityProvider>) -> Result<(), DomainError> {
        self.db
            .transaction(move |tx| {
                tx.execute("DELETE FROM capability_provider", [])
                    .map_err(map_sql_err)?;
                rows.iter().try_for_each(|r| upsert_tx(tx, r))
            })
            .await
    }

    pub async fn list(&self) -> Result<Vec<CapabilityProvider>, DomainError> {
        self.db.call_mut(|c| list_tx(c)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(capability: &str, provider: &str, features: Value) -> CapabilityProvider {
        CapabilityProvider {
            capability: capability.into(),
            provider: provider.into(),
            extension: None,
            features,
            active: true,
        }
    }

    #[tokio::test]
    async fn providers_round_trip_restate_and_reset() {
        let store = SqliteCapabilityStore::new(Database::in_memory());
        store
            .upsert(row("work_items", "oxplow", json!({ "comments": true })))
            .await
            .unwrap();
        let mut fake = row("work_items", "fake", json!({ "comments": false }));
        fake.extension = Some("tracker".into());
        store.upsert(fake.clone()).await.unwrap();
        fake.features = json!({ "comments": true });
        store.upsert(fake.clone()).await.unwrap();
        assert_eq!(
            store.list().await.unwrap(),
            vec![
                fake.clone(),
                row("work_items", "oxplow", json!({ "comments": true }))
            ]
        );
        store.remove("work_items", "fake").await.unwrap();
        assert_eq!(store.list().await.unwrap().len(), 1);
        store
            .reset(vec![row("vcs", "git", json!({ "remotes": true }))])
            .await
            .unwrap();
        assert_eq!(
            store.list().await.unwrap(),
            vec![row("vcs", "git", json!({ "remotes": true }))]
        );
    }
}
