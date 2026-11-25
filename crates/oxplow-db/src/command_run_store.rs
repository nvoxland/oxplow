//! `command_run` (migration V93): every command the bus ran, with who
//! ran it, how it went, the input, and the inverse that undoes it.
//! See `.context/commands.md`.

use std::collections::BTreeMap;

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;

use oxplow_domain::events::schema::{ActorKind, CommandOutcome as Outcome};
use oxplow_domain::{AgentSessionId, CommandCall, DomainError, EventId, ThreadId, Timestamp};

use crate::database::string_to_ts;
use crate::database::{map_sql_err, Database};
use oxplow_domain::run::{Outcome as RunOutcome, RunContext};

pub(crate) fn actor_kind_str(k: ActorKind) -> &'static str {
    match k {
        ActorKind::Human => "human",
        ActorKind::Agent => "agent",
        ActorKind::Page => "page",
        ActorKind::Component => "component",
        ActorKind::System => "system",
        ActorKind::Effect => "effect",
        ActorKind::Job => "job",
    }
}

pub(crate) fn parse_actor_kind(s: &str) -> Result<ActorKind, DomainError> {
    Ok(match s {
        "human" => ActorKind::Human,
        "agent" => ActorKind::Agent,
        "page" => ActorKind::Page,
        "component" => ActorKind::Component,
        "system" => ActorKind::System,
        "effect" => ActorKind::Effect,
        "job" => ActorKind::Job,
        other => return Err(DomainError::Invalid(format!("actor kind `{other}`"))),
    })
}

fn parse_outcome(s: &str) -> Result<Outcome, DomainError> {
    Ok(match s {
        "ok" => Outcome::Ok,
        "denied" => Outcome::Denied,
        "invalid" => Outcome::Invalid,
        "failed" | "skipped" | "cancelled" | "interrupted" => Outcome::Error,
        other => return Err(DomainError::Invalid(format!("outcome `{other}`"))),
    })
}

/// One recorded run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct CommandRun {
    pub id: i64,
    pub at: Timestamp,
    pub command: String,
    pub actor_kind: ActorKind,
    pub actor_id: Option<String>,
    pub thread_id: Option<ThreadId>,
    /// The agent session that ran it, when an agent did through one.
    pub session_id: Option<AgentSessionId>,
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
    /// The command detail ID of the run that undid this one.
    pub undone_by: Option<i64>,
    /// The scopes the run called, and how often.
    pub scopes: BTreeMap<String, u32>,
}

/// What a run writes before its event id is known.
#[derive(Debug, Clone)]
pub struct NewCommandRun {
    pub command: String,
    /// A read or unchanged command keeps no input, result or inverse.
    pub input: Option<Value>,
    pub outcome: Outcome,
    pub error: Option<String>,
    pub result: Option<Value>,
    pub inverse: Option<CommandCall>,
    /// The scopes the command called, as counted by its handlers.
    pub scopes: BTreeMap<String, u32>,
}

/// Record the run and its command details in the caller's transaction.
pub fn insert_tx(
    conn: &Connection,
    run: &RunContext,
    row: &NewCommandRun,
) -> Result<i64, DomainError> {
    let outcome = match row.outcome {
        Outcome::Ok => RunOutcome::Ok,
        Outcome::Denied => RunOutcome::Denied,
        Outcome::Invalid => RunOutcome::Invalid,
        Outcome::Error => RunOutcome::Failed,
    };
    insert_with_outcome_tx(conn, run, row, Some(outcome))
}

/// Write a command with its exact shared outcome.
pub fn insert_with_outcome_tx(
    conn: &Connection,
    run: &RunContext,
    row: &NewCommandRun,
    outcome: Option<RunOutcome>,
) -> Result<i64, DomainError> {
    if !run.contribution_ref().is_some_and(|reference| {
        reference.id.kind == oxplow_domain::contribution::ContributionKind::Command
    }) {
        return Err(DomainError::Invariant(format!(
            "command {} needs its own command run",
            row.command
        )));
    }
    match outcome {
        Some(outcome) => {
            crate::contribution_run_store::record_tx(conn, run, outcome, row.error.as_deref())?
        }
        None => crate::contribution_run_store::begin_tx(conn, run)?,
    }
    if !row.scopes.is_empty() {
        conn.execute(
            "UPDATE contribution_run SET scopes_json = json_patch(scopes_json, ?2) WHERE run_id = ?1",
            params![run.run().as_str(), serde_json::to_string(&row.scopes).expect("scopes serialize")],
        )
        .map_err(map_sql_err)?;
    }
    conn.execute(
        "INSERT INTO command_run (command, run_id, input_json, result_json, inverse_json, device_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            row.command,
            run.run().as_str(),
            row.input.as_ref().map(Value::to_string),
            row.result.as_ref().map(Value::to_string),
            row.inverse
                .as_ref()
                .map(|call| serde_json::to_string(call).expect("CommandCall serializes")),
            run.caller().map(|c| c.device.as_str()),
        ],
    )
    .map_err(map_sql_err)?;
    Ok(conn.last_insert_rowid())
}

/// Keep observations of handlers that ran inside a rolled-back composite.
/// Payloads and inverses belong only to changes that landed.
pub fn insert_rolled_back_children_tx(
    conn: &Connection,
    run: &RunContext,
) -> Result<(), DomainError> {
    for (child, outcome, error) in run.rollback_observations() {
        let Some(contribution) = child.contribution_ref() else {
            continue;
        };
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM contribution_run WHERE run_id = ?1)",
                [child.run().as_str()],
                |row| row.get(0),
            )
            .map_err(map_sql_err)?;
        if exists {
            continue;
        }
        let error = error.as_deref().or_else(|| {
            (outcome == RunOutcome::Cancelled).then_some("parent command transaction rolled back")
        });
        crate::contribution_run_store::record_tx(conn, &child, outcome, error)?;
        conn.execute(
            "INSERT INTO command_run (command, run_id, device_id) VALUES (?1, ?2, ?3)",
            params![
                contribution.id.local,
                child.run().as_str(),
                child.caller().map(|c| c.device.as_str())
            ],
        )
        .map_err(map_sql_err)?;
    }
    Ok(())
}

/// Attach the `command.executed` event to the row, once appended.
pub fn set_event_id_tx(conn: &Connection, id: i64, event_id: &EventId) -> Result<(), DomainError> {
    conn.execute(
        "UPDATE command_run SET event_id = ?2 WHERE id = ?1",
        params![id, event_id.as_str()],
    )
    .map_err(map_sql_err)?;
    Ok(())
}

/// Record that `undone_by` (another command detail row) applied this run's inverse.
pub fn mark_undone_tx(conn: &Connection, id: i64, undone_by: i64) -> Result<(), DomainError> {
    let n = conn
        .execute(
            "UPDATE command_run SET undone_by = ?2 WHERE id = ?1 AND undone_by IS NULL",
            params![id, undone_by],
        )
        .map_err(map_sql_err)?;
    if n == 0 {
        return Err(DomainError::Invariant(format!(
            "command run {id} is missing or already undone"
        )));
    }
    Ok(())
}

fn row_to_command_run(row: &rusqlite::Row<'_>) -> rusqlite::Result<CommandRun> {
    let conv = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    let at: String = row.get("at")?;
    let actor_kind: String = row.get("actor_kind")?;
    let outcome: String = row.get("outcome")?;
    let input: Option<String> = row.get("input_json")?;
    let inverse: Option<String> = row.get("inverse_json")?;
    let result: Option<String> = row.get("result_json")?;
    let event_id: Option<String> = row.get("event_id")?;
    let scopes: Option<String> = row.get("scopes_json")?;
    Ok(CommandRun {
        id: row.get("id")?,
        at: string_to_ts(&at).map_err(conv)?,
        command: row.get("command")?,
        actor_kind: parse_actor_kind(&actor_kind).map_err(conv)?,
        actor_id: row.get("actor_id")?,
        thread_id: row.get::<_, Option<i64>>("thread_id")?.map(ThreadId::new),
        session_id: row
            .get::<_, Option<i64>>("session_id")?
            .map(AgentSessionId::new),
        input: input
            .map(|input| serde_json::from_str(&input))
            .transpose()
            .map_err(|e| conv(DomainError::Storage(format!("input json: {e}"))))?
            .unwrap_or(Value::Null),
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
        scopes: scopes
            .map(|s| serde_json::from_str(&s))
            .transpose()
            .map_err(|e| conv(DomainError::Storage(format!("scopes json: {e}"))))?
            .unwrap_or_default(),
    })
}

const COMMAND_RUN_SELECT: &str =
    "SELECT a.*, r.started_at AS at, r.actor_kind, r.actor_id, r.thread_id, r.session_id,
            r.outcome, r.error, r.scopes_json
     FROM command_run a JOIN contribution_run r ON r.run_id = a.run_id";

pub fn get_tx(conn: &Connection, id: i64) -> Result<Option<CommandRun>, DomainError> {
    conn.query_row(
        &format!("{COMMAND_RUN_SELECT} WHERE a.id = ?1"),
        params![id],
        row_to_command_run,
    )
    .optional()
    .map_err(map_sql_err)
}

/// Newest first.
pub fn list_recent_tx(conn: &Connection, limit: usize) -> Result<Vec<CommandRun>, DomainError> {
    let mut stmt = conn
        .prepare(&format!("{COMMAND_RUN_SELECT} ORDER BY a.id DESC LIMIT ?1"))
        .map_err(map_sql_err)?;
    let rows = stmt
        .query_map(params![limit as i64], row_to_command_run)
        .map_err(map_sql_err)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sql_err)
}

/// Async wrappers over the `_tx` cores. The bus writes through the cores
/// inside its own transaction; these are for reads and tests.
#[derive(Clone)]
pub struct SqliteCommandRunStore {
    db: Database,
}

impl SqliteCommandRunStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn get(&self, id: i64) -> Result<Option<CommandRun>, DomainError> {
        self.db.call_mut(move |c| get_tx(c, id)).await
    }

    pub async fn list_recent(&self, limit: usize) -> Result<Vec<CommandRun>, DomainError> {
        self.db.call_mut(move |c| list_recent_tx(c, limit)).await
    }
}

#[cfg(test)]
pub(crate) fn test_command_run(command: &str, actor: oxplow_domain::Actor) -> RunContext {
    use oxplow_domain::contribution::{ContributionId, ContributionKind};
    use oxplow_domain::run::{ContributionRef, Mode};
    RunContext::contribution(
        ContributionRef {
            id: ContributionId::new("oxplow", ContributionKind::Command, command),
            extension: "core".into(),
            version: None,
            generation: None,
            deployment: None,
        },
        actor,
        Mode::External,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    /// A person's run keeps the device it was asked from, so the audit says
    /// where they acted; a run nobody asked from a device keeps none.
    #[tokio::test]
    async fn a_command_run_keeps_the_device_that_asked() {
        let db = Database::in_memory();
        let device = oxplow_domain::DeviceId::generate();
        let caller = oxplow_domain::Caller {
            device: device.clone(),
            connection: oxplow_domain::ConnectionId::generate(),
        };
        let row = || NewCommandRun {
            command: "oxplow.thread.rename".into(),
            input: None,
            outcome: Outcome::Ok,
            error: None,
            result: None,
            inverse: None,
            scopes: BTreeMap::new(),
        };
        let devices = db
            .transaction(move |tx| {
                let asked = insert_tx(
                    tx,
                    &test_command_run("oxplow.thread.rename", oxplow_domain::Actor::Human)
                        .with_caller(Some(caller.clone())),
                    &row(),
                )?;
                let unasked = insert_tx(
                    tx,
                    &test_command_run("oxplow.thread.rename", oxplow_domain::Actor::Human),
                    &row(),
                )?;
                let device_of = |id: i64| -> Option<String> {
                    tx.query_row(
                        "SELECT device_id FROM command_run WHERE id = ?1",
                        [id],
                        |r| r.get(0),
                    )
                    .unwrap()
                };
                Ok((device_of(asked), device_of(unasked)))
            })
            .await
            .unwrap();
        assert_eq!(devices, (Some(device.to_string()), None));
    }

    /// A child that ran inside a rolled-back run keeps the device too: the
    /// person asked for it as they asked for its parent.
    #[tokio::test]
    async fn a_rolled_back_child_keeps_the_device_that_asked() {
        use oxplow_domain::contribution::{ContributionId, ContributionKind};
        use oxplow_domain::run::{ContributionRef, Mode};
        let db = Database::in_memory();
        let device = oxplow_domain::DeviceId::generate();
        let parent = test_command_run("oxplow.thread.rename", oxplow_domain::Actor::Human)
            .with_caller(Some(oxplow_domain::Caller {
                device: device.clone(),
                connection: oxplow_domain::ConnectionId::generate(),
            }));
        let child = parent.child(
            Some(ContributionRef {
                id: ContributionId::new("oxplow", ContributionKind::Command, "oxplow.thread.mark"),
                extension: "core".into(),
                version: None,
                generation: None,
                deployment: None,
            }),
            Mode::External,
        );
        parent.track_child(&child);
        child.observe_rollback_outcome(oxplow_domain::run::Outcome::Failed, Some("no".into()));
        let recorded: Option<String> = db
            .transaction(move |tx| {
                insert_rolled_back_children_tx(tx, &parent)?;
                Ok(tx
                    .query_row(
                        "SELECT device_id FROM command_run WHERE command = 'oxplow.thread.mark'",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap())
            })
            .await
            .unwrap();
        assert_eq!(recorded, Some(device.to_string()));
    }

    #[tokio::test]
    async fn command_run_rows_round_trip_and_mark_undone_once() {
        let db = Database::in_memory();
        let store = SqliteCommandRunStore::new(db.clone());
        let event = EventId::generate();
        let (id, undo_id) = db
            .transaction({
                let event = event.clone();
                move |tx| {
                    let id = insert_tx(
                        tx,
                        &test_command_run(
                            "oxplow.task.transition",
                            oxplow_domain::Actor::Agent {
                                thread_id: Some(ThreadId::new(3)),
                                session_id: Some(AgentSessionId::new(5)),
                                stream_id: None,
                            },
                        ),
                        &NewCommandRun {
                            command: "oxplow.task.transition".into(),
                            input: Some(json!({"id": "tsk1", "to": "done"})),
                            outcome: Outcome::Ok,
                            error: None,
                            result: Some(json!({"id": "tsk1"})),
                            inverse: Some(CommandCall {
                                name: "oxplow.task.transition".into(),
                                input: json!({"id": "tsk1", "to": "ready"}),
                            }),
                            scopes: [("sql.read".to_string(), 2)].into(),
                        },
                    )?;
                    set_event_id_tx(tx, id, &event)?;
                    let undo_id = insert_tx(
                        tx,
                        &test_command_run("oxplow.task.transition", oxplow_domain::Actor::Human),
                        &NewCommandRun {
                            command: "oxplow.task.transition".into(),
                            input: Some(json!({"id": "tsk1", "to": "ready"})),
                            outcome: Outcome::Ok,
                            error: None,
                            result: None,
                            inverse: None,
                            scopes: BTreeMap::new(),
                        },
                    )?;
                    mark_undone_tx(tx, id, undo_id)?;
                    Ok((id, undo_id))
                }
            })
            .await
            .unwrap();
        let row = store.get(id).await.unwrap().unwrap();
        assert_eq!(row.session_id, Some(AgentSessionId::new(5)));
        assert_eq!(store.get(undo_id).await.unwrap().unwrap().session_id, None);
        assert_eq!(row.command, "oxplow.task.transition");
        assert_eq!(row.actor_kind, ActorKind::Agent);
        assert_eq!(row.thread_id, Some(ThreadId::new(3)));
        assert_eq!(row.outcome, Outcome::Ok);
        assert_eq!(row.event_id, Some(event));
        assert_eq!(row.inverse.as_ref().unwrap().input["to"], "ready");
        assert_eq!(row.result, Some(json!({"id": "tsk1"})));
        assert_eq!(row.undone_by, Some(undo_id));
        assert_eq!(row.scopes, [("sql.read".to_string(), 2)].into());
        assert!(store.get(undo_id).await.unwrap().unwrap().scopes.is_empty());
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
        // A denied run is recorded too, with no inverse.
        db.transaction(|tx| {
            insert_tx(
                tx,
                &test_command_run(
                    "oxplow.config.set",
                    oxplow_domain::Actor::Page {
                        page_id: "acme/x".into(),
                        on_behalf_of: Box::new(oxplow_domain::Actor::Human),
                    },
                ),
                &NewCommandRun {
                    command: "oxplow.config.set".into(),
                    input: Some(json!({})),
                    outcome: Outcome::Denied,
                    error: Some("pages may not run oxplow.config.set".into()),
                    result: None,
                    inverse: None,
                    scopes: BTreeMap::new(),
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
