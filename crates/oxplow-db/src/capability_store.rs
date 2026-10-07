//! `capability_provider` (migration V128, P6b; V15 adds why): each
//! capability's implementations, with the features each declares, which
//! is active and why. Published as `v_capability_provider`, so the UI
//! hides what an implementation can't do and Settings lists the choices.
//! Restated whole by the app's capability registry whenever what it holds
//! or the choices change (`oxplow_app::capabilities`).

use rusqlite::{params, Connection};
use serde_json::Value;

use oxplow_domain::DomainError;

use crate::database::{map_sql_err, Database};

/// One implementation of a capability.
#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityProvider {
    /// `work_items`, `effort_policy`, `snapshots`, `vcs`, `knowledge`.
    pub capability: String,
    /// The implementation's id (`oxplow`, `git`, `issues`, `none`).
    pub provider: String,
    /// The extension declaring it; `None` for core's.
    pub extension: Option<String>,
    /// The features it declares.
    pub features: Value,
    /// Whether it's the capability's active implementation.
    pub active: bool,
    /// How a person names it.
    pub title: String,
    /// How it's loaded: `core`, `builtin`, `external`, `none` — or
    /// `unknown` for a choice nothing provides.
    pub source: String,
    /// Whether it's there to be used: `false` for a chosen one that isn't
    /// (its extension disabled, its instance stopped, its id unknown).
    pub available: bool,
    /// On the active row, why it's the one: `personal`, `project`,
    /// `default` or `fallback`.
    pub chosen_by: Option<String>,
}

fn insert_tx(conn: &Connection, row: &CapabilityProvider) -> Result<(), DomainError> {
    conn.execute(
        "INSERT INTO capability_provider
           (capability, provider, extension, features_json, active, title, source, available, chosen_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            row.capability,
            row.provider,
            row.extension,
            row.features.to_string(),
            row.active,
            row.title,
            row.source,
            row.available,
            row.chosen_by,
        ],
    )
    .map_err(map_sql_err)?;
    Ok(())
}

pub fn list_tx(conn: &Connection) -> Result<Vec<CapabilityProvider>, DomainError> {
    let mut stmt = conn
        .prepare(
            "SELECT capability, provider, extension, features_json, active, title, source,
                    available, chosen_by
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
                title: r.get(5)?,
                source: r.get(6)?,
                available: r.get(7)?,
                chosen_by: r.get(8)?,
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

    /// Replace every row with `rows`.
    pub async fn reset(&self, rows: Vec<CapabilityProvider>) -> Result<(), DomainError> {
        self.db
            .transaction(move |tx| {
                tx.execute("DELETE FROM capability_provider", [])
                    .map_err(map_sql_err)?;
                rows.iter().try_for_each(|r| insert_tx(tx, r))
            })
            .await
    }

    pub async fn list(&self) -> Result<Vec<CapabilityProvider>, DomainError> {
        self.db.read(|tx| list_tx(tx)).await
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
            title: provider.into(),
            source: "builtin".into(),
            available: true,
            chosen_by: Some("default".into()),
        }
    }

    #[tokio::test]
    async fn rows_are_restated_whole() {
        let store = SqliteCapabilityStore::new(Database::in_memory());
        let mut fake = row("work_items", "fake", json!({ "comments": false }));
        fake.extension = Some("tracker".into());
        fake.available = false;
        fake.chosen_by = None;
        store
            .reset(vec![
                row("work_items", "oxplow", json!({ "comments": true })),
                fake.clone(),
            ])
            .await
            .unwrap();
        assert_eq!(
            store.list().await.unwrap(),
            vec![
                fake,
                row("work_items", "oxplow", json!({ "comments": true }))
            ]
        );
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
