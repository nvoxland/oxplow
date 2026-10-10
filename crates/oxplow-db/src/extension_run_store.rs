//! `extension_run` (V48): one row per finished run of extension code
//! (`oxplow_domain::run`, `.context/data-model.md` "extension_run") — its
//! identity, actor, anchors and cause, how it ended, what it cost and what
//! it used. Written by the host that ran it, in the transaction its own
//! record lands in where it has one. A run of core's own work (no
//! contribution behind it) writes nothing. Read as `v_extension_run`;
//! swept on the collector window ([`sweep_tx`]).

use oxplow_domain::run::{Outcome, RunContext};
use oxplow_domain::{DomainError, Timestamp};
use rusqlite::params;

use crate::command_audit_store::actor_kind_str;
use crate::database::map_sql_err;

/// Record `run`, finished with `outcome` (and `error`, when it failed), if
/// it is a contribution's. `decisions` is what its grant decided, as JSON.
pub fn record_tx(
    conn: &rusqlite::Connection,
    run: &RunContext,
    outcome: Outcome,
    error: Option<&str>,
    decisions: &serde_json::Value,
) -> Result<(), DomainError> {
    let Some(contribution) = run.contribution_ref() else {
        return Ok(());
    };
    let usage = run.usage();
    let anchors = run.anchors();
    let actor = run.actor();
    conn.execute(
        "INSERT OR REPLACE INTO extension_run
           (run_id, parent_run_id, extension, contribution, kind, version, generation,
            actor_kind, actor, stream_id, thread_id, effort_id, turn_id, agent_session_id,
            snapshot_id, cause_event_id, mode, started_at, finished_at, outcome, error,
            wall_ms, cpu_ms, peak_bytes, ai_calls, scopes_json, decisions_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                 ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27)",
        params![
            run.run().as_str(),
            run.parent().map(|p| p.as_str().to_string()),
            contribution.id.extension,
            contribution.id.to_string(),
            contribution.id.kind.segment(),
            contribution.version,
            contribution.generation,
            actor_kind_str(actor.kind()),
            actor.source(),
            anchors.stream_id.map(|s| s.value()),
            anchors.thread_id.map(|t| t.value()),
            anchors.effort_id.map(|e| e.value()),
            anchors.turn_id,
            anchors.agent_session_id.map(|s| s.value()),
            anchors.snapshot_id,
            run.cause().map(|c| c.to_string()),
            run.mode().as_str(),
            run.began().to_string(),
            Timestamp::now().to_string(),
            outcome.as_str(),
            error,
            i64::try_from(run.elapsed().as_millis()).unwrap_or(i64::MAX),
            i64::try_from(usage.cpu_ms).unwrap_or(i64::MAX),
            i64::try_from(usage.peak_bytes).unwrap_or(i64::MAX),
            usage.ai_calls,
            serde_json::to_string(&usage.scopes).expect("scopes serialize"),
            decisions.to_string(),
        ],
    )
    .map_err(map_sql_err)?;
    Ok(())
}

/// Delete the runs that began before `cutoff`, at most `limit` of them:
/// how many went.
pub fn sweep_tx(
    conn: &rusqlite::Connection,
    cutoff: &str,
    limit: usize,
) -> Result<usize, DomainError> {
    conn.execute(
        "DELETE FROM extension_run WHERE run_id IN
           (SELECT run_id FROM extension_run WHERE started_at < ?1 LIMIT ?2)",
        params![cutoff, limit as i64],
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
                version: Some("1.0.0".into()),
                generation: Some("g1".into()),
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

    async fn rows(db: &Database) -> Vec<(String, String, Option<i64>, String, i64, String)> {
        db.read(|c| {
            let mut s = c
                .prepare(
                    "SELECT contribution, outcome, thread_id, cause_event_id, cpu_ms, scopes
                     FROM v_extension_run ORDER BY started_at",
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
            record_tx(tx, &run, Outcome::Ok, None, &serde_json::json!([]))?;
            record_tx(tx, &core, Outcome::Ok, None, &serde_json::json!([]))
        })
        .await
        .unwrap();
        assert_eq!(
            rows(&db).await,
            vec![(
                "acme/effect/note".into(),
                "ok".into(),
                Some(3),
                "e1".into(),
                12,
                r#"{"sql.read":1}"#.into()
            )]
        );
    }

    /// The sweep removes runs that began before the cutoff, newer ones stay.
    #[tokio::test]
    async fn the_sweep_removes_old_runs() {
        let db = Database::in_memory();
        let run = effect_run();
        db.transaction(move |tx| {
            record_tx(
                tx,
                &run,
                Outcome::Failed,
                Some("no"),
                &serde_json::json!([]),
            )
        })
        .await
        .unwrap();
        let gone = db
            .transaction(|tx| sweep_tx(tx, "2000-01-01T00:00:00Z", 100))
            .await
            .unwrap();
        assert_eq!(gone, 0);
        let gone = db
            .transaction(|tx| sweep_tx(tx, "9999-01-01T00:00:00Z", 100))
            .await
            .unwrap();
        assert_eq!(gone, 1);
        assert!(rows(&db).await.is_empty());
    }
}
