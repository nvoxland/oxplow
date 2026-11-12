//! Background jobs an agent started (`agent_job`): a background shell or
//! subagent, from the hook that first shows it to the one that reports it.
//! Written by the hook ingest in the transaction of that hook, so a turn
//! opened by a job's report finds the turn it goes on with at once. Read
//! through `v_agent_job`. See `.context/agent-model.md` "Background jobs".

use rusqlite::{params, Connection, OptionalExtension};

use oxplow_domain::hook::CauseBasis;
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
    /// The turn it was started in, as far as this hook knows, and how that
    /// is known (`matched`: a guess from the Stop that listed it).
    pub started_turn: Option<AgentTurnId>,
    pub basis: Option<CauseBasis>,
    /// This hook shows it running (a start, a Stop's list), not reporting:
    /// a job seen running after its report is a new run of it (a subagent
    /// resumed under the same id).
    pub running: bool,
}

/// A recorded job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    pub id: i64,
    pub started_turn: Option<AgentTurnId>,
    pub basis: Option<CauseBasis>,
    pub reported: bool,
}

fn basis_of(s: Option<String>) -> Option<CauseBasis> {
    match s.as_deref() {
        Some("reported") => Some(CauseBasis::Reported),
        Some("matched") => Some(CauseBasis::Matched),
        _ => None,
    }
}

/// The job `id` of agent session `session` on `thread`, when recorded.
pub fn find_tx(
    conn: &Connection,
    thread: ThreadId,
    session: Option<AgentSessionId>,
    id: &str,
) -> Result<Option<Job>, DomainError> {
    conn.query_row(
        "SELECT id, started_turn_id, start_basis, reported_at IS NOT NULL FROM agent_job
          WHERE thread_id = ?1 AND harness_job_id = ?2 AND agent_session_id IS ?3
          ORDER BY id DESC LIMIT 1",
        params![thread.value(), id, session.map(|s| s.value())],
        |r| {
            Ok(Job {
                id: r.get(0)?,
                started_turn: r.get::<_, Option<i64>>(1)?.map(AgentTurnId::new),
                basis: basis_of(r.get(2)?),
                reported: r.get(3)?,
            })
        },
    )
    .optional()
    .map_err(map_sql_err)
}

/// Record that `job` is known, at `at`: a new row the first time (and
/// for a job seen running again after it reported); after that, what the
/// row lacks is filled in, and a reported start replaces a matched guess.
pub fn seen_tx(
    conn: &Connection,
    thread: ThreadId,
    session: Option<AgentSessionId>,
    job: &JobSighting<'_>,
    at: Timestamp,
) -> Result<Job, DomainError> {
    let basis = job.started_turn.and(job.basis);
    match find_tx(conn, thread, session, job.id)? {
        Some(found) if !(found.reported && job.running) => {
            let better = job.started_turn.is_some()
                && (found.started_turn.is_none()
                    || (found.basis == Some(CauseBasis::Matched)
                        && basis == Some(CauseBasis::Reported)));
            let (started_turn, basis) = if better {
                (job.started_turn, basis)
            } else {
                (found.started_turn, found.basis)
            };
            conn.execute(
                "UPDATE agent_job
                    SET kind = coalesce(kind, ?2), description = coalesce(description, ?3),
                        start_call_id = coalesce(start_call_id, ?4),
                        started_turn_id = ?5, start_basis = ?6
                  WHERE id = ?1",
                params![
                    found.id,
                    job.kind,
                    job.description,
                    job.start_call,
                    started_turn.map(|t| t.value()),
                    basis.map(CauseBasis::as_str)
                ],
            )
            .map_err(map_sql_err)?;
            Ok(Job {
                started_turn,
                basis,
                ..found
            })
        }
        _ => {
            conn.execute(
                "INSERT INTO agent_job (thread_id, agent_session_id, harness_job_id, kind,
                                        description, start_call_id, started_turn_id,
                                        start_basis, seen_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    thread.value(),
                    session.map(|s| s.value()),
                    job.id,
                    job.kind,
                    job.description,
                    job.start_call,
                    job.started_turn.map(|t| t.value()),
                    basis.map(CauseBasis::as_str),
                    ts_to_string(at)
                ],
            )
            .map_err(map_sql_err)?;
            Ok(Job {
                id: conn.last_insert_rowid(),
                started_turn: job.started_turn,
                basis,
                reported: false,
            })
        }
    }
}

/// Agent session `session` on `thread` ended or restarted at `at`: its jobs
/// with no report never will report (its process is gone). They are
/// `lost`, oxplow's own fact.
pub fn lose_unreported_tx(
    conn: &Connection,
    thread: ThreadId,
    session: Option<AgentSessionId>,
    at: Timestamp,
) -> Result<usize, DomainError> {
    conn.execute(
        "UPDATE agent_job SET lost_at = ?3
          WHERE thread_id = ?1 AND agent_session_id IS ?2
            AND reported_at IS NULL AND lost_at IS NULL",
        params![thread.value(), session.map(|s| s.value()), ts_to_string(at)],
    )
    .map_err(map_sql_err)
}

/// Every unreported job of every session, at oxplow's start: the processes
/// that owned them are gone (oxplow relaunches its agents), so none will
/// report. They are `lost`, as a session's end loses them.
pub fn lose_all_unreported_tx(conn: &Connection, at: Timestamp) -> Result<usize, DomainError> {
    conn.execute(
        "UPDATE agent_job SET lost_at = ?1 WHERE reported_at IS NULL AND lost_at IS NULL",
        params![ts_to_string(at)],
    )
    .map_err(map_sql_err)
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
