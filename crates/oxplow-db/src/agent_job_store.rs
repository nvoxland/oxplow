//! Background jobs an agent started (`agent_job`): a background shell or
//! subagent, from the hook that first shows it to the one that reports it.
//! Written by the hook ingest in the transaction of that hook, so a turn
//! opened by a job's report finds the turn it goes on with at once. Read
//! through `v_agent_job`. See `.context/agent-model.md` "Background jobs".

use rusqlite::{params, Connection, OptionalExtension};

use oxplow_domain::{AgentSessionId, AgentTurnId, DomainError, ThreadId, Timestamp};

use crate::database::{map_sql_err, ts_to_string};

/// A job as one hook shows it.
#[derive(Debug, Clone, Default)]
pub struct JobSighting<'a> {
    /// The harness's own id for it: the one its report names.
    pub id: &'a str,
    pub kind: Option<&'a str>,
    pub description: Option<&'a str>,
    /// The harness's id of the tool call that started it.
    pub start_call: Option<&'a str>,
    /// The turn it was started in, as far as this hook knows.
    pub started_turn: Option<AgentTurnId>,
}

/// A recorded job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub id: i64,
    pub started_turn: Option<AgentTurnId>,
    pub reported: bool,
}

/// The job `id` of agent session `session` on `thread`, when recorded.
pub fn find_tx(
    conn: &Connection,
    thread: ThreadId,
    session: Option<AgentSessionId>,
    id: &str,
) -> Result<Option<Job>, DomainError> {
    conn.query_row(
        "SELECT id, started_turn_id, reported_at IS NOT NULL FROM agent_job
          WHERE thread_id = ?1 AND harness_job_id = ?2 AND agent_session_id IS ?3
          ORDER BY id DESC LIMIT 1",
        params![thread.value(), id, session.map(|s| s.value())],
        |r| {
            Ok(Job {
                id: r.get(0)?,
                started_turn: r.get::<_, Option<i64>>(1)?.map(AgentTurnId::new),
                reported: r.get(2)?,
            })
        },
    )
    .optional()
    .map_err(map_sql_err)
}

/// Record that `job` is known, at `at`: a new row the first time; after
/// that, what the row lacks is filled in (a later hook may name its kind,
/// its start call or the turn it started in) and nothing it has changes.
pub fn seen_tx(
    conn: &Connection,
    thread: ThreadId,
    session: Option<AgentSessionId>,
    job: &JobSighting<'_>,
    at: Timestamp,
) -> Result<Job, DomainError> {
    if let Some(found) = find_tx(conn, thread, session, job.id)? {
        conn.execute(
            "UPDATE agent_job
                SET kind = coalesce(kind, ?2), description = coalesce(description, ?3),
                    start_call_id = coalesce(start_call_id, ?4),
                    started_turn_id = coalesce(started_turn_id, ?5)
              WHERE id = ?1",
            params![
                found.id,
                job.kind,
                job.description,
                job.start_call,
                job.started_turn.map(|t| t.value())
            ],
        )
        .map_err(map_sql_err)?;
        return Ok(Job {
            started_turn: found.started_turn.or(job.started_turn),
            ..found
        });
    }
    conn.execute(
        "INSERT INTO agent_job (thread_id, agent_session_id, harness_job_id, kind, description,
                                start_call_id, started_turn_id, seen_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            thread.value(),
            session.map(|s| s.value()),
            job.id,
            job.kind,
            job.description,
            job.start_call,
            job.started_turn.map(|t| t.value()),
            ts_to_string(at)
        ],
    )
    .map_err(map_sql_err)?;
    Ok(Job {
        id: conn.last_insert_rowid(),
        started_turn: job.started_turn,
        reported: false,
    })
}

/// Job `job` reported back at `at`, in turn `turn`, with how it ended.
/// A job reports once: a second report (a hand-back, then its notice)
/// keeps the first's turn and time and fills only what the first lacked.
pub fn reported_tx(
    conn: &Connection,
    job: i64,
    turn: Option<AgentTurnId>,
    at: Timestamp,
    status: Option<&str>,
    summary: Option<&str>,
) -> Result<(), DomainError> {
    conn.execute(
        "UPDATE agent_job
            SET report_turn_id = coalesce(report_turn_id, ?2),
                reported_at = coalesce(reported_at, ?3),
                status = coalesce(status, ?4), summary = coalesce(summary, ?5)
          WHERE id = ?1",
        params![
            job,
            turn.map(|t| t.value()),
            ts_to_string(at),
            status,
            summary
        ],
    )
    .map_err(map_sql_err)?;
    Ok(())
}
