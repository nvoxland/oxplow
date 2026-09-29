//! The event log (`event_log`, `event_consumer_checkpoint`,
//! `event_dead_letter`; migration V93). See `.context/data-model.md`
//! "event_log" and `.context/target-architecture.md` §5.
//!
//! **The contract is [`append_tx`]**: a producer appends its envelope
//! inside the same `Database::transaction` closure as the state change
//! it records (the outbox pattern), so the log row is the one write that
//! belongs inside a transaction. The async methods on
//! [`SqliteEventLogStore`] are thin wrappers for callers that have no
//! surrounding transaction (tests, the pump's bookkeeping).
//!
//! Delivery is at least once: a consumer reads `seq > checkpoint`
//! ([`read_after_tx`]), handles the batch, and commits its writes and
//! [`set_checkpoint_tx`] in one transaction. A handler that fails parks
//! the event in the dead-letter table ([`dead_letter_tx`]) and the
//! checkpoint still advances, so one poison event never stalls the pump.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use specta::Type;

use std::sync::Arc;

use oxplow_domain::{
    Anchors, DomainError, EffortId, Envelope, EventId, EventSchemaRegistry, StoredEvent, StreamId,
    ThreadId, Timestamp,
};

use crate::database::{map_sql_err, Database};
use crate::database::{string_to_ts, ts_to_string};

/// Who is writing the events a store's transaction core appends — so a
/// core (`effort_store::start_tx`, …) logs its own change without every
/// caller having to remember, and a command's run can still name itself
/// as the source and cause.
#[derive(Clone)]
pub struct EventCtx<'a> {
    pub schemas: &'a EventSchemaRegistry,
    /// `system:<component>`, or an actor's source when a command runs.
    pub source: String,
    /// The event that caused these (a command's `command.executed`).
    pub cause: Option<EventId>,
}

impl<'a> EventCtx<'a> {
    /// A system component writing on its own behalf (`system:<component>`).
    pub fn system(schemas: &'a EventSchemaRegistry, component: &str) -> Self {
        Self {
            schemas,
            source: oxplow_domain::refs::build::system_source(component),
            cause: None,
        }
    }

    /// A typed envelope carrying this context's source and cause.
    pub fn typed<T: oxplow_domain::events::schema::EventType>(
        &self,
        payload: &T::Payload,
    ) -> Envelope {
        let env = Envelope::typed::<T>(self.source.clone(), payload);
        match &self.cause {
            Some(c) => env.with_cause(c.clone()),
            None => env,
        }
    }

    pub fn append(&self, conn: &Connection, env: &Envelope) -> Result<i64, DomainError> {
        append_tx(conn, self.schemas, env)
    }
}

/// The anchors an event about work on `thread` carries: the thread and
/// its stream (looked up in the same transaction).
pub fn anchors_for_thread_tx(conn: &Connection, thread: ThreadId) -> Result<Anchors, DomainError> {
    let stream: Option<i64> = conn
        .query_row(
            "SELECT stream_id FROM threads WHERE id = ?1",
            params![thread.value()],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sql_err)?;
    Ok(Anchors {
        stream_id: stream.map(StreamId::new),
        thread_id: Some(thread),
        ..Anchors::default()
    })
}

/// Append `env` unless an event with its `dedupe_key` is already logged —
/// for a producer that may see the same fact twice (a re-posted hook).
/// `Ok(false)` when it was already there; nothing is written and the
/// caller's transaction carries on. An envelope with no dedupe key is
/// always appended.
pub fn append_unique_tx(
    conn: &Connection,
    schemas: &EventSchemaRegistry,
    env: &Envelope,
) -> Result<bool, DomainError> {
    Ok(insert_tx(conn, schemas, env, true)?.is_some())
}

/// Append one envelope. Returns its `seq`. The payload must validate
/// against the registered schema for `type@v` ([`DomainError::Invalid`]
/// otherwise, nothing written). A duplicate `dedupe_key` (or `id`) fails
/// with [`DomainError::Constraint`] and writes nothing, which is what lets
/// an at-least-once producer retry blindly.
pub fn append_tx(
    conn: &Connection,
    schemas: &EventSchemaRegistry,
    env: &Envelope,
) -> Result<i64, DomainError> {
    Ok(insert_tx(conn, schemas, env, false)?.expect("a plain insert inserts or fails"))
}

/// The one insert. `skip_duplicate` makes a `dedupe_key` collision a
/// no-op (`None`) decided by the insert itself, so a concurrent twin can't
/// slip between a check and the insert; otherwise it is a `Constraint`.
fn insert_tx(
    conn: &Connection,
    schemas: &EventSchemaRegistry,
    env: &Envelope,
    skip_duplicate: bool,
) -> Result<Option<i64>, DomainError> {
    schemas.validate_envelope(env)?;
    let subject = serde_json::to_string(&env.subject)
        .map_err(|e| DomainError::Invalid(format!("subject: {e}")))?;
    let payload = serde_json::to_string(&env.payload)
        .map_err(|e| DomainError::Invalid(format!("payload: {e}")))?;
    let on_conflict = if skip_duplicate {
        " ON CONFLICT (dedupe_key) DO NOTHING"
    } else {
        ""
    };
    let inserted = conn
        .execute(
            &format!(
            "INSERT INTO event_log
               (id, type, v, at, source, stream_id, thread_id, effort_id, turn_id, snapshot_id,
                subject, payload, payload_hash, cause, dedupe_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15){on_conflict}"
        ),
            params![
                env.id.as_str(),
                env.event_type,
                env.v,
                ts_to_string(env.at),
                env.source,
                env.anchors.stream_id.map(|i| i.value()),
                env.anchors.thread_id.map(|i| i.value()),
                env.anchors.effort_id.map(|i| i.value()),
                env.anchors.turn_id,
                env.anchors.snapshot_id,
                subject,
                payload,
                env.payload_hash,
                env.cause.as_ref().map(|c| c.as_str()),
                env.dedupe_key,
            ],
        )
        .map_err(map_sql_err)?;
    Ok((inserted == 1).then(|| conn.last_insert_rowid()))
}

fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredEvent> {
    let conv = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    let subject: String = row.get("subject")?;
    let payload: String = row.get("payload")?;
    let at: String = row.get("at")?;
    let cause: Option<String> = row.get("cause")?;
    let expired: Option<String> = row.get("payload_expired_at")?;
    Ok(StoredEvent {
        payload_expired_at: expired
            .map(|s| string_to_ts(&s))
            .transpose()
            .map_err(conv)?,
        seq: row.get("seq")?,
        envelope: Envelope {
            id: EventId(row.get("id")?),
            event_type: row.get("type")?,
            v: row.get::<_, i64>("v")? as u32,
            at: string_to_ts(&at).map_err(conv)?,
            source: row.get("source")?,
            anchors: Anchors {
                stream_id: row.get::<_, Option<i64>>("stream_id")?.map(StreamId::new),
                thread_id: row.get::<_, Option<i64>>("thread_id")?.map(ThreadId::new),
                effort_id: row.get::<_, Option<i64>>("effort_id")?.map(EffortId::new),
                turn_id: row.get("turn_id")?,
                snapshot_id: row.get("snapshot_id")?,
            },
            subject: serde_json::from_str(&subject)
                .map_err(|e| conv(DomainError::Storage(format!("subject json: {e}"))))?,
            payload: serde_json::from_str(&payload)
                .map_err(|e| conv(DomainError::Storage(format!("payload json: {e}"))))?,
            payload_hash: row.get("payload_hash")?,
            cause: cause.map(EventId),
            dedupe_key: row.get("dedupe_key")?,
        },
    })
}

/// Events with `seq > after`, oldest first, at most `limit`.
pub fn read_after_tx(
    conn: &Connection,
    after: i64,
    limit: usize,
) -> Result<Vec<StoredEvent>, DomainError> {
    let mut stmt = conn
        .prepare("SELECT * FROM event_log WHERE seq > ?1 ORDER BY seq ASC LIMIT ?2")
        .map_err(map_sql_err)?;
    let rows = stmt
        .query_map(params![after, limit as i64], row_to_event)
        .map_err(map_sql_err)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sql_err)
}

/// One event by id.
pub fn get_tx(conn: &Connection, id: &EventId) -> Result<Option<StoredEvent>, DomainError> {
    conn.query_row(
        "SELECT * FROM event_log WHERE id = ?1",
        params![id.as_str()],
        row_to_event,
    )
    .optional()
    .map_err(map_sql_err)
}

/// The consumer's last handled `seq`; `0` when it has never run.
pub fn checkpoint_tx(conn: &Connection, consumer: &str) -> Result<i64, DomainError> {
    conn.query_row(
        "SELECT last_seq FROM event_consumer_checkpoint WHERE consumer = ?1",
        params![consumer],
        |r| r.get(0),
    )
    .optional()
    .map_err(map_sql_err)
    .map(|v| v.unwrap_or(0))
}

/// Record that `consumer` has handled everything up to `seq`. Commit it
/// in the same transaction as the consumer's own writes.
pub fn set_checkpoint_tx(conn: &Connection, consumer: &str, seq: i64) -> Result<(), DomainError> {
    conn.execute(
        "INSERT INTO event_consumer_checkpoint (consumer, last_seq, updated_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT (consumer) DO UPDATE SET last_seq = excluded.last_seq,
                                             updated_at = excluded.updated_at",
        params![consumer, seq, ts_to_string(Timestamp::now())],
    )
    .map_err(map_sql_err)?;
    Ok(())
}

/// A parked event: the consumer that failed on it, why, and how often.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DeadLetter {
    pub id: i64,
    pub consumer: String,
    pub event_seq: i64,
    pub error: String,
    pub attempts: i64,
    pub first_failed_at: Timestamp,
    pub last_failed_at: Timestamp,
    /// `pending` | `retried` | `discarded`.
    pub state: String,
}

/// Park `event_seq` for `consumer`. A repeat failure of the same pair
/// bumps `attempts` and replaces the error rather than adding a row, and
/// puts a `retried`/`discarded` row back to `pending`.
pub fn dead_letter_tx(
    conn: &Connection,
    consumer: &str,
    event_seq: i64,
    error: &str,
) -> Result<(), DomainError> {
    let now = ts_to_string(Timestamp::now());
    conn.execute(
        "INSERT INTO event_dead_letter
           (consumer, event_seq, error, attempts, first_failed_at, last_failed_at, state)
         VALUES (?1, ?2, ?3, 1, ?4, ?4, 'pending')
         ON CONFLICT (consumer, event_seq) DO UPDATE SET
           error = excluded.error,
           attempts = attempts + 1,
           last_failed_at = excluded.last_failed_at,
           state = 'pending'",
        params![consumer, event_seq, error, now],
    )
    .map_err(map_sql_err)?;
    Ok(())
}

/// Move a dead letter to `retried` or `discarded`.
pub fn set_dead_letter_state_tx(
    conn: &Connection,
    id: i64,
    state: &str,
) -> Result<(), DomainError> {
    let n = conn
        .execute(
            "UPDATE event_dead_letter SET state = ?2 WHERE id = ?1",
            params![id, state],
        )
        .map_err(map_sql_err)?;
    if n == 0 {
        return Err(DomainError::NotFound);
    }
    Ok(())
}

fn row_to_dead_letter(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeadLetter> {
    let conv = |e: DomainError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    };
    let first: String = row.get("first_failed_at")?;
    let last: String = row.get("last_failed_at")?;
    Ok(DeadLetter {
        id: row.get("id")?,
        consumer: row.get("consumer")?,
        event_seq: row.get("event_seq")?,
        error: row.get("error")?,
        attempts: row.get("attempts")?,
        first_failed_at: string_to_ts(&first).map_err(conv)?,
        last_failed_at: string_to_ts(&last).map_err(conv)?,
        state: row.get("state")?,
    })
}

/// Events whose type matches `pattern` (a `LIKE` pattern), newest first,
/// narrowed to a thread or a stream anchor when given.
pub fn recent_tx(
    conn: &Connection,
    pattern: &str,
    thread: Option<ThreadId>,
    stream: Option<StreamId>,
    limit: usize,
) -> Result<Vec<StoredEvent>, DomainError> {
    let (sql, anchor) = match (thread, stream) {
        (Some(t), _) => (
            "SELECT * FROM event_log WHERE thread_id = ?1 AND type LIKE ?2 ORDER BY seq DESC LIMIT ?3",
            Some(t.value()),
        ),
        (None, Some(s)) => (
            "SELECT * FROM event_log WHERE stream_id = ?1 AND type LIKE ?2 ORDER BY seq DESC LIMIT ?3",
            Some(s.value()),
        ),
        (None, None) => (
            "SELECT * FROM event_log WHERE ?1 IS NULL AND type LIKE ?2 ORDER BY seq DESC LIMIT ?3",
            None,
        ),
    };
    let mut stmt = conn.prepare(sql).map_err(map_sql_err)?;
    let rows = stmt
        .query_map(params![anchor, pattern, limit as i64], row_to_event)
        .map_err(map_sql_err)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sql_err)
}

/// Dead letters, `pending` only unless `all`, oldest first.
pub fn list_dead_letters_tx(conn: &Connection, all: bool) -> Result<Vec<DeadLetter>, DomainError> {
    let sql = if all {
        "SELECT * FROM event_dead_letter ORDER BY id ASC"
    } else {
        "SELECT * FROM event_dead_letter WHERE state = 'pending' ORDER BY id ASC"
    };
    let mut stmt = conn.prepare(sql).map_err(map_sql_err)?;
    let rows = stmt
        .query_map([], row_to_dead_letter)
        .map_err(map_sql_err)?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(map_sql_err)
}

/// Async wrappers over the `_tx` cores for callers with no surrounding
/// transaction. Producers should not use `append` for a state change —
/// they compose [`append_tx`] into the change's own transaction.
#[derive(Clone)]
pub struct SqliteEventLogStore {
    db: Database,
    schemas: Arc<EventSchemaRegistry>,
}

impl SqliteEventLogStore {
    pub fn new(db: Database, schemas: Arc<EventSchemaRegistry>) -> Self {
        Self { db, schemas }
    }

    /// The registry `append` validates against; producers composing
    /// [`append_tx`] into their own transaction take it from here.
    pub fn schemas(&self) -> &Arc<EventSchemaRegistry> {
        &self.schemas
    }

    /// Append in a transaction of its own. For activity that has no
    /// state write of its own (an agent tool call, a lens view).
    pub async fn append(&self, env: Envelope) -> Result<i64, DomainError> {
        let schemas = self.schemas.clone();
        self.db
            .transaction(move |tx| append_tx(tx, &schemas, &env))
            .await
    }

    pub async fn read_after(
        &self,
        after: i64,
        limit: usize,
    ) -> Result<Vec<StoredEvent>, DomainError> {
        self.db
            .call_mut(move |conn| read_after_tx(conn, after, limit))
            .await
    }

    /// The newest `limit` events of `namespace` (`agent`, …), newest first:
    /// on `thread`'s anchor when given, else on `stream`'s, else all. What
    /// a thread's status is derived from and the activity log lists.
    pub async fn recent(
        &self,
        namespace: &str,
        thread: Option<ThreadId>,
        stream: Option<StreamId>,
        limit: usize,
    ) -> Result<Vec<StoredEvent>, DomainError> {
        let pattern = format!("{namespace}.%");
        self.db
            .call_mut(move |conn| recent_tx(conn, &pattern, thread, stream, limit))
            .await
    }

    pub async fn get(&self, id: EventId) -> Result<Option<StoredEvent>, DomainError> {
        self.db.call_mut(move |conn| get_tx(conn, &id)).await
    }

    pub async fn checkpoint(&self, consumer: String) -> Result<i64, DomainError> {
        self.db
            .call_mut(move |conn| checkpoint_tx(conn, &consumer))
            .await
    }

    pub async fn set_checkpoint(&self, consumer: String, seq: i64) -> Result<(), DomainError> {
        self.db
            .call_mut(move |conn| set_checkpoint_tx(conn, &consumer, seq))
            .await
    }

    pub async fn dead_letter(
        &self,
        consumer: String,
        event_seq: i64,
        error: String,
    ) -> Result<(), DomainError> {
        self.db
            .call_mut(move |conn| dead_letter_tx(conn, &consumer, event_seq, &error))
            .await
    }

    pub async fn set_dead_letter_state(&self, id: i64, state: String) -> Result<(), DomainError> {
        self.db
            .call_mut(move |conn| set_dead_letter_state_tx(conn, id, &state))
            .await
    }

    pub async fn list_dead_letters(&self, all: bool) -> Result<Vec<DeadLetter>, DomainError> {
        self.db
            .call_mut(move |conn| list_dead_letters_tx(conn, all))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::events::schema::{
        ActorKind, CommandExecuted, CommandExecutedV1, CommandOutcome, ConfigChanged,
        ConfigChangedV1, WorkItemTransitioned, WorkItemTransitionedV1,
    };
    use oxplow_domain::TaskStatus;
    use serde_json::{json, Value};

    fn schemas() -> Arc<EventSchemaRegistry> {
        Arc::new(EventSchemaRegistry::core())
    }

    fn store(db: &Database) -> SqliteEventLogStore {
        SqliteEventLogStore::new(db.clone(), schemas())
    }

    /// A valid `config.changed@1` envelope, optionally with a dedupe key.
    fn env(key: Option<&str>) -> Envelope {
        let e = Envelope::typed::<ConfigChanged>(
            "test",
            &ConfigChangedV1 {
                key: "zones".into(),
                before: Value::Null,
                after: json!([]),
            },
        );
        match key {
            Some(k) => e.with_dedupe_key(k),
            None => e,
        }
    }

    #[tokio::test]
    async fn append_inside_a_rolled_back_transaction_leaves_no_row() {
        let db = Database::in_memory();
        let store = store(&db);
        let schemas = schemas();
        let res = db
            .transaction(move |tx| {
                append_tx(tx, &schemas, &env(None))?;
                Err::<(), _>(DomainError::Invariant("boom".into()))
            })
            .await;
        assert!(res.is_err());
        assert!(store.read_after(0, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn append_commits_with_the_state_change_and_reads_back_in_order() {
        let db = Database::in_memory();
        let store = store(&db);
        let e1 = Envelope::typed::<WorkItemTransitioned>(
            "human",
            &WorkItemTransitionedV1 {
                work_item: "work_item:oxplow:tsk4".into(),
                from: TaskStatus::Ready,
                to: TaskStatus::InProgress,
                effort: Some("effort:eff9".into()),
            },
        )
        .with_dedupe_key("k1")
        .with_anchors(Anchors {
            stream_id: Some(StreamId::new(3)),
            thread_id: Some(ThreadId::new(4)),
            ..Anchors::default()
        })
        .with_subject(["work_item:oxplow:tsk4", "effort:eff9"]);
        let e2 = Envelope::typed::<CommandExecuted>(
            "agent:thr4",
            &CommandExecutedV1 {
                command: "work_item.transition".into(),
                actor_kind: ActorKind::Agent,
                actor_id: Some("thr4".into()),
                outcome: CommandOutcome::Ok,
                audit_id: 1,
                undoable: true,
            },
        )
        .with_cause(e1.id.clone());
        let (s1, s2) = db
            .transaction({
                let (e1, e2, schemas) = (e1.clone(), e2.clone(), schemas());
                move |tx| Ok((append_tx(tx, &schemas, &e1)?, append_tx(tx, &schemas, &e2)?))
            })
            .await
            .unwrap();
        assert!(s2 > s1);
        let all = store.read_after(0, 10).await.unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].seq, s1);
        assert_eq!(all[0].envelope, e1);
        assert_eq!(all[1].envelope, e2);
        assert_eq!(all[1].envelope.cause, Some(e1.id.clone()));
        // Paging by seq.
        let tail = store.read_after(s1, 10).await.unwrap();
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].seq, s2);
        assert_eq!(store.get(e1.id.clone()).await.unwrap().unwrap().seq, s1);
        // `at` is stored in the fixed-width canonical form so it orders.
        let at: String = db
            .call(move |c| {
                c.query_row("SELECT at FROM event_log WHERE seq = ?1", [s1], |r| {
                    r.get(0)
                })
            })
            .await
            .unwrap();
        assert!(
            at.ends_with('Z') && at.len() == "2026-09-28T00:00:00.000000Z".len(),
            "{at}"
        );
    }

    #[tokio::test]
    async fn append_unique_skips_a_duplicate_key_by_the_insert_itself() {
        // The duplicate is caught by the insert (ON CONFLICT), not by a
        // read first, so a concurrent twin can't turn into a Constraint.
        let db = Database::in_memory();
        let schemas = Arc::new(EventSchemaRegistry::core());
        let s2 = schemas.clone();
        let landed = db
            .transaction(move |tx| {
                let env = |key: &str| {
                    Envelope::new(
                        "config.changed",
                        1,
                        "test",
                        serde_json::json!({"key": "k", "before": null, "after": 1}),
                    )
                    .unwrap()
                    .with_dedupe_key(key)
                };
                Ok((
                    append_unique_tx(tx, &s2, &env("once"))?,
                    append_unique_tx(tx, &s2, &env("once"))?,
                    append_unique_tx(tx, &s2, &env("other"))?,
                ))
            })
            .await
            .unwrap();
        assert_eq!(landed, (true, false, true));
        let n = SqliteEventLogStore::new(db, schemas)
            .read_after(0, 10)
            .await
            .unwrap()
            .len();
        assert_eq!(n, 2);
    }

    #[tokio::test]
    async fn a_duplicate_dedupe_key_is_a_constraint_error_and_writes_nothing() {
        let db = Database::in_memory();
        let store = store(&db);
        store.append(env(Some("same"))).await.unwrap();
        let err = store.append(env(Some("same"))).await.unwrap_err();
        assert!(matches!(err, DomainError::Constraint(_)), "{err:?}");
        assert_eq!(store.read_after(0, 10).await.unwrap().len(), 1);
        // No key → no dedupe.
        store.append(env(None)).await.unwrap();
        store.append(env(None)).await.unwrap();
        assert_eq!(store.read_after(0, 10).await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn an_unregistered_type_or_invalid_payload_is_refused_before_the_write() {
        let db = Database::in_memory();
        let store = store(&db);
        let mut unknown = env(None);
        unknown.event_type = "effort.opened".into();
        let err = store.append(unknown).await.unwrap_err();
        assert!(matches!(err, DomainError::Invalid(_)), "{err:?}");
        let mut bad = env(None);
        bad.payload = json!({"n": 1});
        let err = store.append(bad).await.unwrap_err();
        assert!(matches!(err, DomainError::Invalid(_)), "{err:?}");
        assert!(store.read_after(0, 10).await.unwrap().is_empty());
    }

    /// Subjects are canonical refs of registered kinds; anything else is
    /// refused before the write (tsk450).
    #[tokio::test]
    async fn subjects_must_be_canonical_refs_of_registered_kinds() {
        let db = Database::in_memory();
        let store = store(&db);
        for bad in ["zones", "config:", "nope:thing", "commit:xyz", "effort:12"] {
            let err = store
                .append(env(None).with_subject([bad]))
                .await
                .unwrap_err();
            assert!(matches!(err, DomainError::Invalid(_)), "{bad}: {err:?}");
            assert!(err.to_string().contains(bad), "{err}");
        }
        store
            .append(env(None).with_subject([
                "config:zones",
                "stream:str1",
                "work_item:oxplow:tsk4",
            ]))
            .await
            .unwrap();
        assert_eq!(store.read_after(0, 10).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn checkpoints_start_at_zero_and_upsert() {
        let db = Database::in_memory();
        let store = store(&db);
        assert_eq!(store.checkpoint("page_ref".into()).await.unwrap(), 0);
        store.set_checkpoint("page_ref".into(), 5).await.unwrap();
        store.set_checkpoint("page_ref".into(), 9).await.unwrap();
        assert_eq!(store.checkpoint("page_ref".into()).await.unwrap(), 9);
        assert_eq!(store.checkpoint("other".into()).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn dead_letters_park_repeat_and_resolve() {
        let db = Database::in_memory();
        let store = store(&db);
        let seq = store.append(env(None)).await.unwrap();
        store
            .dead_letter("c".into(), seq, "first".into())
            .await
            .unwrap();
        store
            .dead_letter("c".into(), seq, "second".into())
            .await
            .unwrap();
        let pending = store.list_dead_letters(false).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].attempts, 2);
        assert_eq!(pending[0].error, "second");
        assert_eq!(pending[0].state, "pending");
        store
            .set_dead_letter_state(pending[0].id, "discarded".into())
            .await
            .unwrap();
        assert!(store.list_dead_letters(false).await.unwrap().is_empty());
        assert_eq!(
            store.list_dead_letters(true).await.unwrap()[0].state,
            "discarded"
        );
        // A later failure reopens it.
        store
            .dead_letter("c".into(), seq, "third".into())
            .await
            .unwrap();
        assert_eq!(store.list_dead_letters(false).await.unwrap()[0].attempts, 3);
        // The FK refuses a letter for an event that doesn't exist.
        let err = store
            .dead_letter("c".into(), 999, "x".into())
            .await
            .unwrap_err();
        assert!(matches!(err, DomainError::Constraint(_)), "{err:?}");
        assert!(matches!(
            store
                .set_dead_letter_state(12345, "retried".into())
                .await
                .unwrap_err(),
            DomainError::NotFound
        ));
    }
}
