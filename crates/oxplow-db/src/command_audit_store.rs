//! `command_audit` (migration V93): every command the bus ran, with who
//! ran it, how it went, the input, and the inverse that undoes it.
//! See `.context/commands.md`.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use oxplow_domain::events::schema::{ActorKind, CommandOutcome as Outcome};
use oxplow_domain::{CommandCall, DomainError, EventId, ThreadId, Timestamp};

use crate::database::{map_sql_err, Database};
use crate::database::{string_to_ts, ts_to_string};

pub(crate) fn actor_kind_str(k: ActorKind) -> &'static str {
    match k {
        ActorKind::Human => "human",
        ActorKind::Agent => "agent",
        ActorKind::Lens => "lens",
        ActorKind::System => "system",
        ActorKind::Effect => "effect",
    }
}

pub(crate) fn parse_actor_kind(s: &str) -> Result<ActorKind, DomainError> {
    Ok(match s {
        "human" => ActorKind::Human,
        "agent" => ActorKind::Agent,
        "lens" => ActorKind::Lens,
        "system" => ActorKind::System,
        "effect" => ActorKind::Effect,
        other => return Err(DomainError::Invalid(format!("actor kind `{other}`"))),
    })
}

fn outcome_str(o: Outcome) -> &'static str {
    match o {
        Outcome::Ok => "ok",
        Outcome::Denied => "denied",
        Outcome::Invalid => "invalid",
        Outcome::Error => "error",
    }
}

fn parse_outcome(s: &str) -> Result<Outcome, DomainError> {
    Ok(match s {
        "ok" => Outcome::Ok,
        "denied" => Outcome::Denied,
        "invalid" => Outcome::Invalid,
        "error" => Outcome::Error,
        other => return Err(DomainError::Invalid(format!("outcome `{other}`"))),
    })
}

/// One audited run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct CommandAudit {
    pub id: i64,
    pub at: Timestamp,
    pub command: String,
    pub actor_kind: ActorKind,
    pub actor_id: Option<String>,
    pub thread_id: Option<ThreadId>,
    pub input: Value,
    pub outcome: Outcome,
    pub error: Option<String>,
    /// What the run returned (a successful run's result); `None` for a
    /// refused or failed one.
    pub result: Option<Value>,
    /// The `command.executed` event, once appended.
    pub event_id: Option<EventId>,
    /// The call that undoes this run, when the command is undoable.
    pub inverse: Option<CommandCall>,
    /// The audit row of the run that undid this one.
    pub undone_by: Option<i64>,
}

/// What a run writes before its event id is known.
#[derive(Debug, Clone)]
pub struct NewCommandAudit {
    pub command: String,
    pub actor_kind: ActorKind,
    pub actor_id: Option<String>,
    pub thread_id: Option<ThreadId>,
    pub input: Value,
    pub outcome: Outcome,
    pub error: Option<String>,
    pub result: Option<Value>,
    pub inverse: Option<CommandCall>,
}

/// Insert one audit row; returns its id.
pub fn insert_tx(conn: &Connection, row: &NewCommandAudit) -> Result<i64, DomainError> {
    let inverse = row
        .inverse
        .as_ref()
        .map(|c| serde_json::to_string(c).expect("CommandCall serializes"));
    conn.execute(
        "INSERT INTO command_audit
           (at, command, actor_kind, actor_id, thread_id, input_json, outcome, error, inverse_json,
            result_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            ts_to_string(Timestamp::now()),
            row.command,
            actor_kind_str(row.actor_kind),
            row.actor_id,
            row.thread_id.map(|t| t.value()),
            serde_json::to_string(&row.input).expect("input serializes"),
            outcome_str(row.outcome),
            row.error,
            inverse,
            row.result
                .as_ref()
                .map(|r| serde_json::to_string(r).expect("result serializes")),
        ],
    )
    .map_err(map_sql_err)?;
    Ok(conn.last_insert_rowid())
}

/// Attach the `command.executed` event to the row, once appended.
pub fn set_event_id_tx(conn: &Connection, id: i64, event_id: &EventId) -> Result<(), DomainError> {
    conn.execute(
        "UPDATE command_audit SET event_id = ?2 WHERE id = ?1",
        params![id, event_id.as_str()],
    )
    .map_err(map_sql_err)?;
    Ok(())
}

/// Record that `undone_by` (another audit row) applied this run's inverse.
pub fn mark_undone_tx(conn: &Connection, id: i64, undone_by: i64) -> Result<(), DomainError> {
    let n = conn
        .execute(
            "UPDATE command_audit SET undone_by = ?2 WHERE id = ?1 AND undone_by IS NULL",
            params![id, undone_by],
        )
        .map_err(map_sql_err)?;
    if n == 0 {
        return Err(DomainError::Invariant(format!(
            "audit row {id} is missing or already undone"
        )));
    }
    Ok(())
}

fn row_to_audit(row: &rusqlite::Row<'_>) -> rusqlite::Result<CommandAudit> {
    let conv = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    let at: String = row.get("at")?;
    let actor_kind: String = row.get("actor_kind")?;
    let outcome: String = row.get("outcome")?;
    let input: String = row.get("input_json")?;
    let inverse: Option<String> = row.get("inverse_json")?;
    let result: Option<String> = row.get("result_json")?;
    let event_id: Option<String> = row.get("event_id")?;
    Ok(CommandAudit {
        id: row.get("id")?,
        at: string_to_ts(&at).map_err(conv)?,
        command: row.get("command")?,
        actor_kind: parse_actor_kind(&actor_kind).map_err(conv)?,
        actor_id: row.get("actor_id")?,
        thread_id: row.get::<_, Option<i64>>("thread_id")?.map(ThreadId::new),
        input: serde_json::from_str(&input)
            .map_err(|e| conv(DomainError::Storage(format!("input json: {e}"))))?,
        outcome: parse_outcome(&outcome).map_err(conv)?,
        error: row.get("error")?,
        result: result
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| conv(DomainError::Storage(format!("result json: {e}"))))?,
        event_id: event_id.map(EventId),
        inverse: inverse
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| conv(DomainError::Storage(format!("inverse json: {e}"))))?,
        undone_by: row.get("undone_by")?,
    })
}

pub fn get_tx(conn: &Connection, id: i64) -> Result<Option<CommandAudit>, DomainError> {
    conn.query_row(
        "SELECT * FROM command_audit WHERE id = ?1",
        params![id],
        row_to_audit,
    )
    .optional()
    .map_err(map_sql_err)
}

/// Newest first.
pub fn list_recent_tx(conn: &Connection, limit: usize) -> Result<Vec<CommandAudit>, DomainError> {
    let mut stmt = conn
        .prepare("SELECT * FROM command_audit ORDER BY id DESC LIMIT ?1")
        .map_err(map_sql_err)?;
    let rows = stmt
        .query_map(params![limit as i64], row_to_audit)
        .map_err(map_sql_err)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sql_err)
}

/// Async wrappers over the `_tx` cores. The bus writes through the cores
/// inside its own transaction; these are for reads and tests.
#[derive(Clone)]
pub struct SqliteCommandAuditStore {
    db: Database,
}

impl SqliteCommandAuditStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn get(&self, id: i64) -> Result<Option<CommandAudit>, DomainError> {
        self.db.call_mut(move |c| get_tx(c, id)).await
    }

    pub async fn list_recent(&self, limit: usize) -> Result<Vec<CommandAudit>, DomainError> {
        self.db.call_mut(move |c| list_recent_tx(c, limit)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    #[tokio::test]
    async fn audit_rows_round_trip_and_mark_undone_once() {
        let db = Database::in_memory();
        let store = SqliteCommandAuditStore::new(db.clone());
        let event = EventId::generate();
        let (id, undo_id) = db
            .transaction({
                let event = event.clone();
                move |tx| {
                    let id = insert_tx(
                        tx,
                        &NewCommandAudit {
                            command: "work_item.transition".into(),
                            actor_kind: ActorKind::Agent,
                            actor_id: Some("thr3".into()),
                            thread_id: Some(ThreadId::new(3)),
                            input: json!({"id": "tsk1", "to": "done"}),
                            outcome: Outcome::Ok,
                            error: None,
                            result: Some(json!({"id": "tsk1"})),
                            inverse: Some(CommandCall {
                                name: "work_item.transition".into(),
                                input: json!({"id": "tsk1", "to": "ready"}),
                            }),
                        },
                    )?;
                    set_event_id_tx(tx, id, &event)?;
                    let undo_id = insert_tx(
                        tx,
                        &NewCommandAudit {
                            command: "work_item.transition".into(),
                            actor_kind: ActorKind::Human,
                            actor_id: None,
                            thread_id: None,
                            input: json!({"id": "tsk1", "to": "ready"}),
                            outcome: Outcome::Ok,
                            error: None,
                            result: None,
                            inverse: None,
                        },
                    )?;
                    mark_undone_tx(tx, id, undo_id)?;
                    Ok((id, undo_id))
                }
            })
            .await
            .unwrap();
        let row = store.get(id).await.unwrap().unwrap();
        assert_eq!(row.command, "work_item.transition");
        assert_eq!(row.actor_kind, ActorKind::Agent);
        assert_eq!(row.thread_id, Some(ThreadId::new(3)));
        assert_eq!(row.outcome, Outcome::Ok);
        assert_eq!(row.event_id, Some(event));
        assert_eq!(row.inverse.as_ref().unwrap().input["to"], "ready");
        assert_eq!(row.result, Some(json!({"id": "tsk1"})));
        assert_eq!(row.undone_by, Some(undo_id));
        // Newest first; a second undo of the same row is refused.
        let recent = store.list_recent(10).await.unwrap();
        assert_eq!(
            recent.iter().map(|r| r.id).collect::<Vec<_>>(),
            vec![undo_id, id]
        );
        let err = db
            .transaction(move |tx| mark_undone_tx(tx, id, undo_id))
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Invariant(_)), "{err:?}");
        // A denied run is audited too, with no inverse.
        db.transaction(|tx| {
            insert_tx(
                tx,
                &NewCommandAudit {
                    command: "config.set".into(),
                    actor_kind: ActorKind::Lens,
                    actor_id: Some("acme/x".into()),
                    thread_id: None,
                    input: json!({}),
                    outcome: Outcome::Denied,
                    error: Some("lenses may not run config.set".into()),
                    result: None,
                    inverse: None,
                },
            )
        })
        .await
        .unwrap();
        assert_eq!(
            store.list_recent(1).await.unwrap()[0].outcome,
            Outcome::Denied
        );
    }
}
