//! `provider_collector_state` (V129, P7.A3): where each provider
//! instance's collector left off — its opaque `$/state` cursor, how many
//! records reads have delivered, and the last read's outcome. A batch of
//! records and the checkpoint covering it commit together
//! ([`checkpoint_tx`]), so a read that fails midway resumes from the last
//! batch that landed.

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use oxplow_domain::DomainError;

use crate::database::{map_sql_err, Database};

/// One collector's row.
#[derive(Debug, Clone, PartialEq)]
pub struct CollectorState {
    pub instance: String,
    pub collector: String,
    /// The provider's last checkpoint; `None` before any.
    pub state: Option<Value>,
    /// `never`, `reading`, `ok` or `error`.
    pub status: String,
    pub error: Option<String>,
    /// RFC 3339.
    pub last_read_at: Option<String>,
    pub records: i64,
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<CollectorState> {
    let state: Option<String> = r.get(2)?;
    Ok(CollectorState {
        instance: r.get(0)?,
        collector: r.get(1)?,
        state: state.and_then(|s| serde_json::from_str(&s).ok()),
        status: r.get(3)?,
        error: r.get(4)?,
        last_read_at: r.get(5)?,
        records: r.get(6)?,
    })
}

const COLUMNS: &str = "instance, collector, state_json, status, error, last_read_at, records";

pub fn get_tx(
    conn: &Connection,
    instance: &str,
    collector: &str,
) -> Result<Option<CollectorState>, DomainError> {
    conn.query_row(
        &format!(
            "SELECT {COLUMNS} FROM provider_collector_state WHERE instance = ?1 AND collector = ?2"
        ),
        params![instance, collector],
        row,
    )
    .optional()
    .map_err(map_sql_err)
}

/// Record `records` more delivered and, when given, the checkpoint that
/// covers them — in the caller's transaction, beside the records.
pub fn checkpoint_tx(
    conn: &Connection,
    instance: &str,
    collector: &str,
    state: Option<&Value>,
    records: i64,
    at: &str,
) -> Result<(), DomainError> {
    conn.execute(
        "INSERT INTO provider_collector_state
            (instance, collector, state_json, status, last_read_at, records)
         VALUES (?1, ?2, ?3, 'reading', ?4, ?5)
         ON CONFLICT (instance, collector) DO UPDATE SET
            state_json = coalesce(excluded.state_json, state_json),
            status = 'reading', last_read_at = excluded.last_read_at,
            records = records + excluded.records",
        params![
            instance,
            collector,
            state.map(Value::to_string),
            at,
            records
        ],
    )
    .map_err(map_sql_err)?;
    Ok(())
}

/// The read finished: `ok`, or `error` with why.
pub fn finish_tx(
    conn: &Connection,
    instance: &str,
    collector: &str,
    error: Option<&str>,
    at: &str,
) -> Result<(), DomainError> {
    conn.execute(
        "INSERT INTO provider_collector_state (instance, collector, status, error, last_read_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (instance, collector) DO UPDATE SET
            status = excluded.status, error = excluded.error,
            last_read_at = excluded.last_read_at",
        params![
            instance,
            collector,
            if error.is_some() { "error" } else { "ok" },
            error,
            at
        ],
    )
    .map_err(map_sql_err)?;
    Ok(())
}

#[derive(Clone)]
pub struct SqliteProviderCollectorStore {
    db: Database,
}

impl SqliteProviderCollectorStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn get(
        &self,
        instance: &str,
        collector: &str,
    ) -> Result<Option<CollectorState>, DomainError> {
        let (i, c) = (instance.to_string(), collector.to_string());
        self.db.read(move |tx| get_tx(tx, &i, &c)).await
    }

    /// Every collector of `instance`, by name.
    pub async fn for_instance(&self, instance: &str) -> Result<Vec<CollectorState>, DomainError> {
        let i = instance.to_string();
        self.db
            .read(move |tx| {
                let mut stmt = tx
                    .prepare(&format!(
                        "SELECT {COLUMNS} FROM provider_collector_state WHERE instance = ?1 \
                         ORDER BY collector"
                    ))
                    .map_err(map_sql_err)?;
                let rows = stmt
                    .query_map([i], row)
                    .and_then(|r| r.collect::<rusqlite::Result<Vec<_>>>())
                    .map_err(map_sql_err)?;
                Ok(rows)
            })
            .await
    }

    pub async fn finish(
        &self,
        instance: &str,
        collector: &str,
        error: Option<String>,
        at: String,
    ) -> Result<(), DomainError> {
        let (i, c) = (instance.to_string(), collector.to_string());
        self.db
            .transaction(move |tx| finish_tx(tx, &i, &c, error.as_deref(), &at))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A checkpoint keeps the latest state and adds up the records; a
    /// batch without one keeps the previous state.
    #[tokio::test]
    async fn checkpoints_accumulate_and_keep_the_latest_state() {
        let db = Database::in_memory();
        let store = SqliteProviderCollectorStore::new(db.clone());
        db.transaction(|tx| {
            checkpoint_tx(
                tx,
                "t/fake",
                "items",
                Some(&serde_json::json!({ "c": 1 })),
                2,
                "a",
            )?;
            checkpoint_tx(tx, "t/fake", "items", None, 1, "b")
        })
        .await
        .unwrap();
        store
            .finish("t/fake", "items", None, "c".into())
            .await
            .unwrap();
        let row = store.get("t/fake", "items").await.unwrap().unwrap();
        assert_eq!(row.state, Some(serde_json::json!({ "c": 1 })));
        assert_eq!((row.records, row.status.as_str()), (3, "ok"));
        assert_eq!(row.last_read_at.as_deref(), Some("c"));
        store
            .finish("t/fake", "items", Some("boom".into()), "d".into())
            .await
            .unwrap();
        let row = store.get("t/fake", "items").await.unwrap().unwrap();
        assert_eq!(
            (row.status.as_str(), row.error.as_deref()),
            ("error", Some("boom"))
        );
        assert_eq!(store.for_instance("t/fake").await.unwrap().len(), 1);
    }
}
