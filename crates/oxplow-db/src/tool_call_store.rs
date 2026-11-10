//! Persisted agent tool calls (`agent_tool_call`), a projection of the
//! tool events: a call's row opens at its `agent.tool.requested` (running,
//! or denied), closes at its `agent.tool.finished`, and is abandoned when
//! its turn ends with it open. Read through `v_tool_call` and
//! `v_context_read`; the bundled review policy derives its struggle signal
//! from them. See `.context/semantic-layer.md`.

use oxplow_domain::DomainError;

use crate::Database;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct NewToolCall {
    pub thread_id: i64,
    pub effort_id: Option<i64>,
    /// The turn it ran in (`agent_turn.id`).
    pub turn_id: Option<i64>,
    /// The harness's own name for the tool.
    pub tool: String,
    /// What it does, in oxplow's vocabulary (`ToolKind::as_str`).
    pub kind: String,
    pub path: Option<String>,
    pub detail: Option<String>,
    pub ok: Option<bool>,
    /// The `agent.tool.finished` event this row projects; a second row for
    /// the same event is not written.
    pub event_id: Option<String>,
    /// When it ran; now when absent.
    pub at: Option<oxplow_domain::Timestamp>,
    /// The agent session that made it (`agent_session.id`).
    pub agent_session_id: Option<i64>,
    /// The subagent it ran inside, when one is known.
    pub subagent_id: Option<String>,
    pub subagent_kind: Option<String>,
    /// The harness's id for the call.
    pub call_id: Option<String>,
    /// How long it ran, as the harness timed it (a finish).
    pub duration_ms: Option<u64>,
}

#[derive(Clone)]
pub struct SqliteToolCallStore {
    db: Database,
}

impl SqliteToolCallStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn record(&self, call: NewToolCall) -> Result<(), DomainError> {
        self.db
            .transaction(move |tx| record_tx(tx, &call).map(|_| ()))
            .await
    }
}

/// Write `call` in the caller's transaction. `false` when a row for its
/// `event_id` already exists (a redelivered event).
pub fn record_tx(conn: &rusqlite::Connection, call: &NewToolCall) -> Result<bool, DomainError> {
    let at = crate::database::ts_to_string(call.at.unwrap_or_else(oxplow_domain::Timestamp::now));
    let n = conn
        .execute(
            "INSERT INTO agent_tool_call
               (thread_id, effort_id, turn_id, tool, path, detail, ok, at, event_id, kind,
                agent_session_id, subagent_id, subagent_kind)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT (event_id) WHERE event_id IS NOT NULL DO NOTHING",
            rusqlite::params![
                call.thread_id,
                call.effort_id,
                call.turn_id,
                call.tool,
                call.path,
                call.detail,
                call.ok.map(i64::from),
                at,
                call.event_id,
                call.kind,
                call.agent_session_id,
                call.subagent_id,
                call.subagent_kind,
            ],
        )
        .map_err(crate::database::map_sql_err)?;
    Ok(n == 1)
}

/// A call was requested (`request_event`): its row opens, `running`, or
/// `denied` (and ended) when the policy refused it. Nothing when the row
/// already exists (a redelivered event).
pub fn record_request_tx(
    conn: &rusqlite::Connection,
    call: &NewToolCall,
    request_event: &str,
    denied: bool,
) -> Result<bool, DomainError> {
    let at = crate::database::ts_to_string(call.at.unwrap_or_else(oxplow_domain::Timestamp::now));
    let n = conn
        .execute(
            "INSERT INTO agent_tool_call
               (thread_id, effort_id, turn_id, tool, path, detail, at, kind, agent_session_id,
                subagent_id, subagent_kind, call_id, started_at, ended_at, state, request_event_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?7,
                     CASE WHEN ?13 THEN ?7 END, CASE WHEN ?13 THEN 'denied' ELSE 'running' END, ?14)
             ON CONFLICT (request_event_id) WHERE request_event_id IS NOT NULL DO NOTHING",
            rusqlite::params![
                call.thread_id,
                call.effort_id,
                call.turn_id,
                call.tool,
                call.path,
                call.detail,
                at,
                call.kind,
                call.agent_session_id,
                call.subagent_id,
                call.subagent_kind,
                call.call_id,
                denied,
                request_event,
            ],
        )
        .map_err(crate::database::map_sql_err)?;
    Ok(n == 1)
}

/// A call finished (`call.event_id`, the finish event): the row its
/// `request_event` opened closes, `finished`, with what the finish
/// reported and how long it ran (the harness's timing, else since its
/// request). With no such row, one is written with no start.
pub fn record_finish_tx(
    conn: &rusqlite::Connection,
    call: &NewToolCall,
    request_event: Option<&str>,
) -> Result<bool, DomainError> {
    let at = crate::database::ts_to_string(call.at.unwrap_or_else(oxplow_domain::Timestamp::now));
    if let Some(request) = request_event {
        let n = conn
            .execute(
                "UPDATE agent_tool_call
                    SET state = 'finished', event_id = ?2, at = ?3, ended_at = ?3, ok = ?4,
                        tool = ?5, kind = ?6, path = ?7, detail = ?8,
                        call_id = coalesce(?9, call_id),
                        subagent_id = coalesce(?10, subagent_id),
                        subagent_kind = coalesce(?11, subagent_kind),
                        duration_ms = coalesce(?12, CAST(round((julianday(?3) - julianday(started_at))
                                                               * 86400000) AS INTEGER))
                  WHERE request_event_id = ?1",
                rusqlite::params![
                    request,
                    call.event_id,
                    at,
                    call.ok.map(i64::from),
                    call.tool,
                    call.kind,
                    call.path,
                    call.detail,
                    call.call_id,
                    call.subagent_id,
                    call.subagent_kind,
                    call.duration_ms.map(|d| d as i64),
                ],
            )
            .map_err(crate::database::map_sql_err)?;
        if n > 0 {
            return Ok(true);
        }
    }
    let n = conn
        .execute(
            "INSERT INTO agent_tool_call
               (thread_id, effort_id, turn_id, tool, path, detail, ok, at, event_id, kind,
                agent_session_id, subagent_id, subagent_kind, call_id, ended_at, duration_ms,
                state, request_event_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?8, ?15,
                     'finished', ?16)
             ON CONFLICT (event_id) WHERE event_id IS NOT NULL DO NOTHING",
            rusqlite::params![
                call.thread_id,
                call.effort_id,
                call.turn_id,
                call.tool,
                call.path,
                call.detail,
                call.ok.map(i64::from),
                at,
                call.event_id,
                call.kind,
                call.agent_session_id,
                call.subagent_id,
                call.subagent_kind,
                call.call_id,
                call.duration_ms.map(|d| d as i64),
                request_event,
            ],
        )
        .map_err(crate::database::map_sql_err)?;
    Ok(n == 1)
}

/// What owned the calls left running: the turn that ended, or the agent
/// session that ended (a call made with no turn open, a background
/// subagent's after the Stop, has only its session).
#[derive(Debug, Clone, Copy)]
pub enum CallOwner {
    Turn(i64),
    Session(i64),
}

/// `owner` ended at `at` with calls still running: they are `abandoned`,
/// ended then, with no duration (they never finished).
pub fn abandon_running_tx(
    conn: &rusqlite::Connection,
    owner: CallOwner,
    at: oxplow_domain::Timestamp,
) -> Result<usize, DomainError> {
    let at = crate::database::ts_to_string(at);
    let (column, id) = match owner {
        CallOwner::Turn(t) => ("turn_id", t),
        CallOwner::Session(s) => ("agent_session_id", s),
    };
    conn.execute(
        &format!(
            "UPDATE agent_tool_call SET state = 'abandoned', ended_at = ?2
              WHERE {column} = ?1 AND state = 'running'"
        ),
        rusqlite::params![id, at],
    )
    .map_err(crate::database::map_sql_err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SemanticLayer;
    use serde_json::json;

    async fn seeded() -> (SqliteToolCallStore, SemanticLayer) {
        let db = Database::in_memory();
        db.call(|c| {
            c.execute_batch(
                "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
                   VALUES (1, 'primary', 'p', 'main', 'refs/heads/main', 'local', '/tmp/x', '2026-01-01', '2026-01-01');
                 INSERT INTO threads (id, stream_id, title, created_at, updated_at)
                   VALUES (1, 1, 'T', '2026-01-01', '2026-01-01');
                 INSERT INTO oxtask (id, title, status, priority, created_by, created_at, updated_at)
                   VALUES (1, 'Task', 'in_progress', 'medium', 'agent', '2026-01-01', '2026-01-01');
                 INSERT INTO effort (id, task, thread_id, started_at) VALUES (1, 'oxplow:oxplow/task?id=tsk1&provider=oxtask', 1, '2026-01-01');",
            )
        })
        .await
        .unwrap();
        (SqliteToolCallStore::new(db.clone()), SemanticLayer::new(db))
    }

    /// A call of `kind` (oxplow's vocabulary), named by its kind.
    fn call(kind: &str, path: Option<&str>, ok: Option<bool>) -> NewToolCall {
        NewToolCall {
            thread_id: 1,
            effort_id: Some(1),
            tool: kind.into(),
            kind: kind.into(),
            path: path.map(str::to_string),
            detail: None,
            ok,
            ..Default::default()
        }
    }

    /// P3.2 (tsk472): the row is a projection of `agent.tool.finished`, so
    /// a redelivered event writes nothing.
    /// The session and the subagent a call ran in are recorded and read
    /// back through `v_tool_call`.
    #[tokio::test]
    async fn a_call_carries_its_session_and_subagent() {
        let (store, sl) = seeded().await;
        store
            .record(NewToolCall {
                agent_session_id: Some(3),
                subagent_id: Some("afd8".into()),
                subagent_kind: Some("Explore".into()),
                ..call("read", Some("a.rs"), Some(true))
            })
            .await
            .unwrap();
        let rows = sl
            .query_sql(
                "SELECT agent_session_id, subagent_id, subagent_kind FROM v_tool_call",
                vec![],
                None,
            )
            .await
            .unwrap()
            .rows;
        assert_eq!(
            serde_json::to_value(rows).unwrap(),
            json!([[3, "afd8", "Explore"]])
        );
    }

    #[tokio::test]
    async fn record_tx_projects_each_event_once() {
        let (store, sl) = seeded().await;
        let call = NewToolCall {
            event_id: Some("evt-1".into()),
            turn_id: None,
            ..call("read", Some("src/a.rs"), Some(true))
        };
        let first = store
            .db
            .transaction({
                let call = call.clone();
                move |tx| record_tx(tx, &call)
            })
            .await
            .unwrap();
        let again = store
            .db
            .transaction(move |tx| record_tx(tx, &call))
            .await
            .unwrap();
        assert_eq!((first, again), (true, false));
        let rows = sl
            .query_sql("SELECT event_id, turn_id FROM v_tool_call", vec![], None)
            .await
            .unwrap()
            .rows;
        assert_eq!(
            serde_json::to_value(rows).unwrap(),
            json!([["evt-1", null]])
        );
    }

    #[tokio::test]
    async fn context_reads_are_derived() {
        let (store, sl) = seeded().await;
        store
            .record(call("read", Some(".context/usability.md"), Some(true)))
            .await
            .unwrap();
        store
            .record(call("read", Some("src/main.rs"), Some(true)))
            .await
            .unwrap();
        let rows = |sql: &'static str| {
            let sl = sl.clone();
            async move {
                serde_json::to_value(sl.query_sql(sql, vec![], None).await.unwrap().rows).unwrap()
            }
        };
        assert_eq!(
            rows("SELECT path FROM v_context_read").await,
            json!([[".context/usability.md"]])
        );
        assert_eq!(rows("SELECT count(*) FROM v_tool_call").await, json!([[2]]));
    }

    /// A turn's time splits into its tool calls (the union of their
    /// intervals: two calls overlap), its waits on the person (each to the
    /// next status, the last cut at the turn's end) and the rest, with no
    /// time counted twice; a denied call never ran.
    #[tokio::test]
    async fn a_turns_time_splits_into_tools_waits_and_the_rest() {
        let db = Database::in_memory();
        let at = |s: u32| format!("2026-01-01T00:00:{s:02}.000000Z");
        let mut seed = format!(
            "INSERT INTO streams (id, kind, title, branch, branch_ref, branch_source, worktree_path, created_at, updated_at)
               VALUES (1, 'primary', 'a', 'main', 'r', 'r', '/r', '{a}', '{a}');
             INSERT INTO threads (id, stream_id, title, created_at, updated_at)
               VALUES (1, 1, 't', '{a}', '{a}');
             INSERT INTO agent_turn (id, thread_id, prompt, started_at, ended_at)
               VALUES (1, 1, 'go', '{a}', '2026-01-01T00:01:00.000000Z');",
            a = at(0)
        );
        for (state, start, end) in [
            ("finished", 10, 30),
            ("finished", 20, 40),
            ("denied", 45, 45),
        ] {
            seed += &format!(
                "INSERT INTO agent_tool_call (thread_id, turn_id, tool, at, kind, state, started_at, ended_at)
                   VALUES (1, 1, 'Bash', '{e}', 'shell', '{state}', '{s}', '{e}');",
                s = at(start),
                e = at(end)
            );
        }
        for (i, (state, when)) in [
            ("running", 0),
            ("awaiting_user", 42),
            ("running", 50),
            ("awaiting_user", 55),
        ]
        .into_iter()
        .enumerate()
        {
            seed += &format!(
                "INSERT INTO event_log (id, type, v, at, source, turn_id, subject, payload)
                   VALUES ('s{i}', 'agent.status.changed', 2, '{t}', 'test', 1, '[]',
                           '{{\"state\":\"{state}\"}}');",
                t = at(when)
            );
        }
        db.transaction(move |tx| {
            tx.execute_batch(&seed)
                .map_err(crate::database::map_sql_err)
        })
        .await
        .unwrap();
        let row: (i64, i64, i64, i64, i64) = db
            .read(|c| {
                c.query_row(
                    "SELECT duration_ms, tool_ms, waits, wait_ms, other_ms FROM v_turn_activity",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
                )
                .map_err(crate::database::map_sql_err)
            })
            .await
            .unwrap();
        // Tools 10–40 (30 s); waits 42–50 and 55–60 (13 s); the rest is
        // 60 less the 43 s either covered.
        assert_eq!(row, (60_000, 30_000, 2, 13_000, 17_000));
    }
}
