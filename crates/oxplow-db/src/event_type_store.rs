//! `event_type_contract` (V144, P8.D3): every event type the vocabulary
//! registered, by `type@v`, with the schema it was first registered with.
//! The app's `vocabulary_reactor` refuses a declared type whose schema
//! differs from the recorded one, then restates the table from what it
//! registered.

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;

use crate::database::map_sql_err;
use oxplow_domain::DomainError;

/// One registered `type@v`.
#[derive(Debug, Clone, PartialEq)]
pub struct EventTypeRow {
    pub event_type: String,
    pub v: u32,
    /// `None` for a core type.
    pub extension: Option<String>,
    pub schema: Value,
    pub summary: Option<String>,
}

/// The schema recorded for `type@v`, if it was ever registered.
pub fn recorded_schema_tx(
    conn: &Connection,
    event_type: &str,
    v: u32,
) -> Result<Option<Value>, DomainError> {
    let text: Option<String> = conn
        .query_row(
            "SELECT schema_json FROM event_type_contract WHERE event_type = ?1 AND v = ?2",
            params![event_type, v],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sql_err)?;
    text.map(|t| {
        serde_json::from_str(&t)
            .map_err(|e| DomainError::Storage(format!("event_type_contract schema: {e}")))
    })
    .transpose()
}

/// Restate the table from the running vocabulary: each of `registered` is
/// recorded (a new one) or marked registered, every other row unmarked. A
/// declared type keeps the schema first recorded (the caller refused a
/// changed one); a core type's follows its golden.
pub fn restate_tx(
    conn: &Connection,
    registered: &[EventTypeRow],
    now: &str,
) -> Result<(), DomainError> {
    conn.execute("UPDATE event_type_contract SET registered = 0", [])
        .map_err(map_sql_err)?;
    let mut st = conn
        .prepare(
            "INSERT INTO event_type_contract
                 (event_type, v, extension, schema_json, summary, registered, recorded_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6)
             ON CONFLICT (event_type, v) DO UPDATE SET
                 registered = 1,
                 extension = excluded.extension,
                 summary = excluded.summary,
                 schema_json = CASE WHEN excluded.extension IS NULL
                                    THEN excluded.schema_json ELSE schema_json END",
        )
        .map_err(map_sql_err)?;
    for row in registered {
        st.execute(params![
            row.event_type,
            row.v,
            row.extension,
            row.schema.to_string(),
            row.summary,
            now,
        ])
        .map_err(map_sql_err)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;
    use serde_json::json;

    fn row(extension: Option<&str>, schema: Value) -> EventTypeRow {
        EventTypeRow {
            event_type: "acme.thing".into(),
            v: 1,
            extension: extension.map(str::to_string),
            schema,
            summary: None,
        }
    }

    #[tokio::test]
    async fn a_declared_schema_is_kept_and_unregistered_rows_stay_listed() {
        let db = Database::in_memory();
        db.transaction(|tx| {
            restate_tx(tx, &[row(Some("acme"), json!({"a": 1}))], "t1")?;
            // A later restate can't move a declared schema …
            restate_tx(tx, &[row(Some("acme"), json!({"b": 2}))], "t2")?;
            assert_eq!(
                recorded_schema_tx(tx, "acme.thing", 1)?,
                Some(json!({"a": 1}))
            );
            // … and one without it leaves the row, unregistered.
            restate_tx(tx, &[], "t3")?;
            let registered: i64 = tx
                .query_row("SELECT registered FROM event_type_contract", [], |r| {
                    r.get(0)
                })
                .map_err(map_sql_err)?;
            assert_eq!(registered, 0);
            // A core type's schema follows its golden.
            restate_tx(tx, &[row(None, json!({"c": 3}))], "t4")?;
            assert_eq!(
                recorded_schema_tx(tx, "acme.thing", 1)?,
                Some(json!({"c": 3}))
            );
            Ok(())
        })
        .await
        .unwrap();
    }
}
