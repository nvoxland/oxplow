//! `effect_run` (V149, P8.D10): each effect's reaction to each event, at
//! most one. The command bus writes the row in a run's own transaction
//! (or claims it `started` before a run with a step outside it); the
//! app's `effect_triggers` reads it to never run, or re-send, twice.

use rusqlite::{params, Connection, OptionalExtension};

use crate::database::map_sql_err;
use oxplow_domain::DomainError;

/// Which reaction: effect `<extension>/<id>` to event `event_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectRunKey {
    pub effect: String,
    pub event_id: String,
    pub event_seq: i64,
}

/// Where a reaction stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Started,
    Ok,
    Skipped,
    Proposed,
    Failed,
}

impl RunState {
    pub fn as_str(self) -> &'static str {
        match self {
            RunState::Started => "started",
            RunState::Ok => "ok",
            RunState::Skipped => "skipped",
            RunState::Proposed => "proposed",
            RunState::Failed => "failed",
        }
    }

    fn parse(s: &str) -> Result<Self, DomainError> {
        Ok(match s {
            "started" => RunState::Started,
            "ok" => RunState::Ok,
            "skipped" => RunState::Skipped,
            "proposed" => RunState::Proposed,
            "failed" => RunState::Failed,
            other => return Err(DomainError::Storage(format!("effect_run state `{other}`"))),
        })
    }
}

/// How a reaction ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    pub state: RunState,
    pub reason: Option<String>,
    pub audit_id: Option<i64>,
    pub proposal_id: Option<i64>,
}

/// The state of `key`'s reaction, if there is one.
pub fn state_tx(conn: &Connection, key: &EffectRunKey) -> Result<Option<RunState>, DomainError> {
    let state: Option<String> = conn
        .query_row(
            "SELECT state FROM effect_run WHERE effect = ?1 AND event_id = ?2",
            params![key.effect, key.event_id],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sql_err)?;
    state.as_deref().map(RunState::parse).transpose()
}

/// Claim `key` before a run with a step outside the transaction: a
/// `started` row. `Invalid` when the reaction already has one.
pub fn claim_tx(conn: &Connection, key: &EffectRunKey, now: &str) -> Result<(), DomainError> {
    let inserted = conn
        .execute(
            "INSERT INTO effect_run (effect, event_id, event_seq, state, started_at)
             VALUES (?1, ?2, ?3, 'started', ?4)
             ON CONFLICT (effect, event_id) DO NOTHING",
            params![key.effect, key.event_id, key.event_seq, now],
        )
        .map_err(map_sql_err)?;
    if inserted == 0 {
        return Err(DomainError::Invalid(format!(
            "effect `{}` already reacted to event {}",
            key.effect, key.event_id
        )));
    }
    Ok(())
}

/// Record how `key`'s reaction ended: its row, or its `started` claim
/// finished. `Invalid` when it already ended (a concurrent or earlier
/// delivery got there first).
pub fn finish_tx(
    conn: &Connection,
    key: &EffectRunKey,
    done: &Finished,
    now: &str,
) -> Result<(), DomainError> {
    match state_tx(conn, key)? {
        Some(RunState::Started) => {
            conn.execute(
                "UPDATE effect_run
                    SET state = ?3, reason = ?4, audit_id = ?5, proposal_id = ?6, finished_at = ?7
                  WHERE effect = ?1 AND event_id = ?2",
                params![
                    key.effect,
                    key.event_id,
                    done.state.as_str(),
                    done.reason,
                    done.audit_id,
                    done.proposal_id,
                    now
                ],
            )
            .map_err(map_sql_err)?;
        }
        Some(_) => {
            return Err(DomainError::Invalid(format!(
                "effect `{}` already reacted to event {}",
                key.effect, key.event_id
            )))
        }
        None => {
            conn.execute(
                "INSERT INTO effect_run
                     (effect, event_id, event_seq, state, reason, audit_id, proposal_id,
                      started_at, finished_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
                params![
                    key.effect,
                    key.event_id,
                    key.event_seq,
                    done.state.as_str(),
                    done.reason,
                    done.audit_id,
                    done.proposal_id,
                    now
                ],
            )
            .map_err(map_sql_err)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    fn key() -> EffectRunKey {
        EffectRunKey {
            effect: "acme/notify".into(),
            event_id: "e1".into(),
            event_seq: 3,
        }
    }

    fn ok() -> Finished {
        Finished {
            state: RunState::Ok,
            reason: None,
            audit_id: Some(9),
            proposal_id: None,
        }
    }

    #[tokio::test]
    async fn a_reaction_ends_once_whether_claimed_first_or_not() {
        let db = Database::in_memory();
        db.transaction(|tx| {
            finish_tx(tx, &key(), &ok(), "t")?;
            assert_eq!(state_tx(tx, &key())?, Some(RunState::Ok));
            assert!(finish_tx(tx, &key(), &ok(), "t").is_err());
            assert!(claim_tx(tx, &key(), "t").is_err());
            let other = EffectRunKey {
                event_id: "e2".into(),
                ..key()
            };
            claim_tx(tx, &other, "t")?;
            assert_eq!(state_tx(tx, &other)?, Some(RunState::Started));
            finish_tx(tx, &other, &ok(), "t")?;
            assert_eq!(state_tx(tx, &other)?, Some(RunState::Ok));
            Ok(())
        })
        .await
        .unwrap();
    }
}
