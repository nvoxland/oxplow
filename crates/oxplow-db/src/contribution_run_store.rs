//! `contribution_run`: one row per run of extension code
//! (`oxplow_domain::run`, `.context/data-model.md` "extension_run") — its
//! identity, actor, anchors and cause, how it ended, what it cost and what
//! it used. Written by the host that ran it, in the transaction its own
//! record lands in where it has one. A run of core's own work (no
//! contribution behind it) writes nothing. Read as `v_contribution_run`;
//! swept on the collector window ([`crate::event_retention::sweep`]).

use oxplow_domain::run::{Outcome, RunContext};
use oxplow_domain::{DomainError, Timestamp};
use rusqlite::params;

use crate::command_audit_store::actor_kind_str;
use crate::database::map_sql_err;

/// Register a contribution run before its work starts.
pub(crate) fn begin_tx(conn: &rusqlite::Connection, run: &RunContext) -> Result<(), DomainError> {
    let Some(contribution) = run.contribution_ref() else {
        return Ok(());
    };
    let anchors = run.anchors();
    let actor = run.actor();
    conn.execute(
        "INSERT INTO contribution_run
           (run_id, parent_run_id, extension, contribution, kind, version, generation,
            actor_kind, actor_id, stream_id, thread_id, turn_id, session_id,
            snapshot_id, cause_event_id, mode, started_at)
         VALUES (?1, (SELECT run_id FROM contribution_run WHERE run_id = ?2),
                 ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
        params![
            run.run().as_str(),
            run.parent().map(|p| p.as_str()),
            contribution.extension,
            contribution.id.to_string(),
            contribution.id.kind.segment(),
            contribution.version,
            contribution.generation,
            actor_kind_str(actor.kind()),
            actor.id(),
            anchors.stream_id.map(|s| s.value()),
            anchors.thread_id.map(|t| t.value()),
            anchors.turn_id,
            anchors.agent_session_id.map(|s| s.value()),
            anchors.snapshot_id,
            run.cause().map(|c| c.to_string()),
            run.mode().as_str(),
            run.began().to_string(),
        ],
    )
    .map_err(map_sql_err)?;
    Ok(())
}

/// Finish a contribution run that was previously registered and remains live.
pub(crate) fn finish_tx(
    conn: &rusqlite::Connection,
    run: &RunContext,
    outcome: Outcome,
    error: Option<&str>,
) -> Result<(), DomainError> {
    finish_at_tx(conn, run, outcome, error, Timestamp::now())
}

pub(crate) fn finish_at_tx(
    conn: &rusqlite::Connection,
    run: &RunContext,
    outcome: Outcome,
    error: Option<&str>,
    finished_at: Timestamp,
) -> Result<(), DomainError> {
    if run.contribution_ref().is_none() {
        return Ok(());
    }
    let usage = run.usage();
    let elapsed = usage.observed_wall_time.unwrap_or_else(|| run.elapsed());
    let updated = conn
        .execute(
            "UPDATE contribution_run
         SET finished_at = ?2, outcome = ?3, error = ?4, wall_ms = ?5,
             cpu_ms = ?6, peak_bytes = ?7, ai_calls = ?8, scopes_json = ?9, decisions_json = ?10
         WHERE run_id = ?1 AND finished_at IS NULL",
            params![
                run.run().as_str(),
                finished_at.to_string(),
                outcome.as_str(),
                error,
                i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX),
                i64::try_from(usage.cpu_ms).unwrap_or(i64::MAX),
                i64::try_from(usage.peak_bytes).unwrap_or(i64::MAX),
                usage.ai_calls,
                serde_json::to_string(&usage.scopes).expect("scopes serialize"),
                serde_json::to_string(&run.decisions()).expect("decisions serialize"),
            ],
        )
        .map_err(map_sql_err)?;
    if updated == 0 {
        return Err(DomainError::Invariant(format!(
            "run {} is missing or already finished",
            run.run()
        )));
    }
    for child in run.recorded_children() {
        conn.execute(
            "UPDATE contribution_run SET parent_run_id = ?2 WHERE run_id = ?1 AND parent_run_id IS NULL",
            params![child.as_str(), run.run().as_str()],
        )
        .map_err(map_sql_err)?;
    }
    Ok(())
}

/// Record a contribution run that begins and ends in the same transaction.
pub(crate) fn record_tx(
    conn: &rusqlite::Connection,
    run: &RunContext,
    outcome: Outcome,
    error: Option<&str>,
) -> Result<(), DomainError> {
    begin_tx(conn, run)?;
    finish_tx(conn, run, outcome, error)
}

/// Write a run whose contribution kind has no detail ledger.
pub fn record_unlogged_tx(
    conn: &rusqlite::Connection,
    run: &RunContext,
    outcome: Option<Outcome>,
    error: Option<&str>,
) -> Result<(), DomainError> {
    use oxplow_domain::contribution::ContributionKind;

    if run.contribution_ref().is_some_and(|reference| {
        matches!(
            reference.id.kind,
            ContributionKind::Command
                | ContributionKind::Collector
                | ContributionKind::Effect
                | ContributionKind::Job
        )
    }) {
        return Err(DomainError::Invalid(
            "write a run with a detail ledger through its kind store".into(),
        ));
    }
    match outcome {
        Some(outcome) => record_tx(conn, run, outcome, error),
        None => begin_tx(conn, run),
    }
}

/// Close live runs left behind by the previous daemon. The effect ledger
/// and job ledgers recover their own claims so they can emit results and
/// enforce their retry and lease rules.
pub fn interrupt_unfinished_tx(conn: &rusqlite::Connection) -> Result<usize, DomainError> {
    conn.execute(
        "UPDATE contribution_run
         SET finished_at = ?1, outcome = ?2, error = 'oxplow stopped before the run finished'
         WHERE finished_at IS NULL
           AND NOT EXISTS (SELECT 1 FROM effect_run WHERE run_id = contribution_run.run_id)
           AND NOT EXISTS (SELECT 1 FROM extension_job_attempt WHERE run_id = contribution_run.run_id)",
        params![Timestamp::now().to_string(), Outcome::Interrupted.as_str()],
    )
    .map_err(map_sql_err)
}

#[cfg(test)]
mod tests {
    use oxplow_domain::contribution::{ContributionId, ContributionKind};
    use oxplow_domain::run::{ContributionRef, Mode};
    use oxplow_domain::{Actor, Anchors, EventId, ThreadId};

    use super::*;
    use crate::database::Database;

    fn effect_run() -> RunContext {
        RunContext::contribution(
            ContributionRef {
                id: ContributionId::new("acme", ContributionKind::Effect, "note"),
                extension: "acme".into(),
                version: Some("1.0.0".into()),
                generation: Some("g1".into()),
                deployment: None,
            },
            Actor::System,
            Mode::External,
        )
        .caused_by(EventId("e1".into()))
        .anchored(Anchors {
            thread_id: Some(ThreadId::new(3)),
            turn_id: Some(7),
            ..Anchors::default()
        })
    }

    #[test]
    fn contribution_run_begins_live_and_finishes_once() {
        let db = Database::in_memory();
        let run = effect_run();
        db.transaction_now(|tx| {
            assert!(finish_tx(tx, &run, Outcome::Ok, None).is_err());
            begin_tx(tx, &run)?;
            let live: bool = tx
                .query_row(
                    "SELECT finished_at IS NULL AND outcome IS NULL AND wall_ms IS NULL
                        AND cpu_ms IS NULL AND peak_bytes IS NULL AND ai_calls IS NULL
                 FROM contribution_run WHERE run_id = ?1",
                    [run.run().as_str()],
                    |row| row.get(0),
                )
                .map_err(map_sql_err)?;
            assert!(live);
            run.used_cpu(9, 2048);
            finish_tx(tx, &run, Outcome::Invalid, Some("bad input"))?;
            assert!(finish_tx(tx, &run, Outcome::Ok, None).is_err());
            let ended: (String, String, i64) = tx
                .query_row(
                    "SELECT outcome, error, cpu_ms FROM contribution_run WHERE run_id = ?1",
                    [run.run().as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .map_err(map_sql_err)?;
            assert_eq!(ended, ("invalid".into(), "bad input".into(), 9));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn contribution_run_reconcile_only_finishes_live_rows() {
        let db = Database::in_memory();
        let live = effect_run();
        let finished = effect_run();
        db.transaction_now(|tx| {
            begin_tx(tx, &live)?;
            record_tx(tx, &finished, Outcome::Ok, None)?;
            assert_eq!(interrupt_unfinished_tx(tx)?, 1);
            assert_eq!(interrupt_unfinished_tx(tx)?, 0);
            let interrupted: bool = tx
                .query_row(
                    "SELECT outcome = 'interrupted' AND finished_at IS NOT NULL AND cpu_ms IS NULL
                 FROM contribution_run WHERE run_id = ?1",
                    [live.run().as_str()],
                    |row| row.get(0),
                )
                .map_err(map_sql_err)?;
            assert!(interrupted);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn a_parent_accounts_for_a_child_that_finished_first() {
        let db = Database::in_memory();
        let parent = effect_run();
        let child = parent.core_command_child(
            ContributionRef {
                id: ContributionId::new("example", ContributionKind::Command, "example.item.write"),
                extension: "example".into(),
                version: None,
                generation: None,
                deployment: None,
            },
            Mode::Transactional,
        );
        parent.track_child(&child);
        parent.used_cpu(5, 512);
        child.used_cpu(9, 1024);
        db.transaction_now(|tx| {
            record_tx(tx, &child, Outcome::Ok, None)?;
            record_tx(tx, &parent, Outcome::Ok, None)?;
            let totals: (i64, i64) = tx.query_row(
                "SELECT sum(cpu_ms), max(peak_bytes) FROM contribution_run WHERE parent_run_id IS NULL",
                [], |row| Ok((row.get(0)?, row.get(1)?)),
            ).map_err(map_sql_err)?;
            assert_eq!(totals, (14, 1024));
            let child_cost: i64 = tx.query_row(
                "SELECT cpu_ms FROM contribution_run WHERE parent_run_id = ?1",
                [parent.run().as_str()], |row| row.get(0),
            ).map_err(map_sql_err)?;
            assert_eq!(child_cost, 9);
            Ok(())
        }).unwrap();
    }

    async fn rows(db: &Database) -> Vec<(String, String, Option<i64>, String, i64, String)> {
        db.read(|c| {
            let mut s = c
                .prepare(
                    "SELECT contribution, outcome, thread_id, cause_event_id, cpu_ms, scopes
                     FROM v_contribution_run ORDER BY started_at",
                )
                .map_err(map_sql_err)?;
            let rows = s
                .query_map([], |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                    ))
                })
                .map_err(map_sql_err)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(map_sql_err)?;
            Ok(rows)
        })
        .await
        .unwrap()
    }

    /// A contribution's run is one row with what it was and what it used;
    /// core's own work writes none.
    #[tokio::test]
    async fn a_contributions_run_is_recorded_and_cores_isnt() {
        let db = Database::in_memory();
        let run = effect_run();
        run.used_scope("sql.read");
        run.used_cpu(12, 4096);
        let core = RunContext::core(Actor::Human, Mode::External);
        db.transaction(move |tx| {
            record_tx(tx, &run, Outcome::Ok, None)?;
            record_tx(tx, &core, Outcome::Ok, None)
        })
        .await
        .unwrap();
        assert_eq!(
            rows(&db).await,
            vec![(
                "oxplow:oxplow/effect?effect=acme/note".into(),
                "ok".into(),
                Some(3),
                "e1".into(),
                12,
                r#"{"sql.read":1}"#.into()
            )]
        );
    }
}
