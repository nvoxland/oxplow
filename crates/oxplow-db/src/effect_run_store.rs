//! `effect_run` (V149, P8.D10; attempts V156, P9.D4): each effect's
//! reaction to each event. The live consumer makes at most one attempt;
//! a person's retry of a failed reaction makes the next, so a reaction
//! is `(effect, event_id)` and its **state is its latest attempt's**.
//! The command bus writes an attempt's row in its run's own transaction
//! (or claims it `started` before a run with a step outside it); the
//! app's `effect_triggers` reads it to never run, or re-send, twice.

use rusqlite::{params, Connection, OptionalExtension};

use crate::database::map_sql_err;
use oxplow_domain::DomainError;

/// What started an attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReactionOrigin {
    /// The pump delivering the event.
    Live,
    /// A person's `effect.retry` of a reaction that failed.
    Retry,
    /// A person's `effect.backfill` over events the effect never saw.
    Backfill,
    /// Sent again by itself (P10): a failed attempt whose every step went
    /// to a provider that keeps `idempotent_writes`.
    Auto,
}

impl ReactionOrigin {
    pub fn as_str(self) -> &'static str {
        match self {
            ReactionOrigin::Live => "live",
            ReactionOrigin::Retry => "retry",
            ReactionOrigin::Backfill => "backfill",
            ReactionOrigin::Auto => "auto",
        }
    }

    pub fn parse(s: &str) -> Result<Self, DomainError> {
        match s {
            "live" => Ok(ReactionOrigin::Live),
            "retry" => Ok(ReactionOrigin::Retry),
            "backfill" => Ok(ReactionOrigin::Backfill),
            "auto" => Ok(ReactionOrigin::Auto),
            other => Err(DomainError::Invariant(format!(
                "an effect_run origin `{other}` isn't one oxplow writes"
            ))),
        }
    }
}

/// Which attempt: effect `<extension>/<id>`'s `attempt`-th at reacting
/// to event `event_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectRunKey {
    pub effect: String,
    pub event_id: String,
    pub event_seq: i64,
    /// From 1.
    pub attempt: u32,
    pub origin: ReactionOrigin,
}

impl EffectRunKey {
    /// The first attempt at a reaction, from `origin`.
    pub fn first(
        effect: impl Into<String>,
        event_id: impl Into<String>,
        event_seq: i64,
        origin: ReactionOrigin,
    ) -> Self {
        Self {
            effect: effect.into(),
            event_id: event_id.into(),
            event_seq,
            attempt: 1,
            origin,
        }
    }
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

/// A reaction's latest attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Latest {
    pub attempt: u32,
    pub state: RunState,
    /// What started it.
    pub origin: ReactionOrigin,
    /// A failed one sent again by itself at this time (RFC 3339).
    pub retry_at: Option<String>,
    /// What that retry sends: the failed attempt's composition, as
    /// [`schedule_retry_tx`] kept it.
    pub resend: Option<String>,
}

/// The reaction of `effect` to `event_id` as it stands: its latest
/// attempt, if it was ever attempted.
pub fn latest_tx(
    conn: &Connection,
    effect: &str,
    event_id: &str,
) -> Result<Option<Latest>, DomainError> {
    let latest = conn
        .query_row(
            "SELECT attempt, state, origin, retry_at, resend_json FROM effect_run
              WHERE effect = ?1 AND event_id = ?2 ORDER BY attempt DESC LIMIT 1",
            params![effect, event_id],
            |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get(3)?,
                    r.get(4)?,
                ))
            },
        )
        .optional()
        .map_err(map_sql_err)?;
    latest
        .map(|(attempt, state, origin, retry_at, resend)| {
            Ok(Latest {
                attempt,
                state: RunState::parse(&state)?,
                origin: ReactionOrigin::parse(&origin)?,
                retry_at,
                resend,
            })
        })
        .transpose()
}

/// How many of the reaction's latest attempts in a row were automatic
/// (P10: at most two are made).
pub fn automatic_in_a_row_tx(
    conn: &Connection,
    effect: &str,
    event_id: &str,
) -> Result<u32, DomainError> {
    let mut st = conn
        .prepare(
            "SELECT origin FROM effect_run WHERE effect = ?1 AND event_id = ?2
              ORDER BY attempt DESC",
        )
        .map_err(map_sql_err)?;
    let origins: Vec<String> = st
        .query_map(params![effect, event_id], |r| r.get(0))
        .and_then(|rows| rows.collect::<rusqlite::Result<_>>())
        .map_err(map_sql_err)?;
    Ok(origins.iter().take_while(|o| o.as_str() == "auto").count() as u32)
}

/// Send `key`'s failed attempt again by itself at `at` (RFC 3339): the
/// automatic attempt sends `resend`, what the failed one composed
/// (tsk887).
pub fn schedule_retry_tx(
    conn: &Connection,
    key: &EffectRunKey,
    at: &str,
    resend: &str,
) -> Result<(), DomainError> {
    conn.execute(
        "UPDATE effect_run SET retry_at = ?4, resend_json = ?5
          WHERE effect = ?1 AND event_id = ?2 AND attempt = ?3 AND state = 'failed'",
        params![key.effect, key.event_id, key.attempt, at, resend],
    )
    .map_err(map_sql_err)?;
    Ok(())
}

/// Drop `key`'s scheduled retry: it won't be sent again by itself.
pub fn drop_retry_tx(conn: &Connection, key: &EffectRunKey) -> Result<(), DomainError> {
    conn.execute(
        "UPDATE effect_run SET retry_at = NULL, resend_json = NULL
          WHERE effect = ?1 AND event_id = ?2 AND attempt = ?3",
        params![key.effect, key.event_id, key.attempt],
    )
    .map_err(map_sql_err)?;
    Ok(())
}

/// Drop every scheduled retry of `effect` (tsk990): what they would send
/// was composed by the script as it was, and a new approval is of the
/// script as it is now. The attempts stay failed, for a person's Retry.
pub fn drop_retries_of_tx(conn: &Connection, effect: &str) -> Result<usize, DomainError> {
    conn.execute(
        "UPDATE effect_run SET retry_at = NULL, resend_json = NULL
          WHERE effect = ?1 AND retry_at IS NOT NULL",
        params![effect],
    )
    .map_err(map_sql_err)
}

/// The failed latest attempts due to be sent again by `now` (RFC 3339),
/// oldest due first, each with when it was due.
pub fn due_retries_tx(
    conn: &Connection,
    now: &str,
) -> Result<Vec<(EffectRunKey, String)>, DomainError> {
    let mut st = conn
        .prepare(
            "SELECT r.effect, r.event_id, r.event_seq, r.attempt, r.origin, r.retry_at
               FROM effect_run r
              WHERE r.state = 'failed' AND r.retry_at IS NOT NULL AND r.retry_at <= ?1
                AND r.attempt = (SELECT max(l.attempt) FROM effect_run l
                                  WHERE l.effect = r.effect AND l.event_id = r.event_id)
              ORDER BY r.retry_at",
        )
        .map_err(map_sql_err)?;
    let rows: Vec<(String, String, i64, u32, String, String)> = st
        .query_map([now], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .and_then(|rows| rows.collect::<rusqlite::Result<_>>())
        .map_err(map_sql_err)?;
    rows.into_iter()
        .map(|(effect, event_id, event_seq, attempt, origin, retry_at)| {
            Ok((
                EffectRunKey {
                    effect,
                    event_id,
                    event_seq,
                    attempt,
                    origin: ReactionOrigin::parse(&origin)?,
                },
                retry_at,
            ))
        })
        .collect()
}

/// The attempts a person started — a retry, a backfill — that are still
/// `started`. No pump delivery makes them, so none redelivers them: at
/// start, each was cut off (tsk845).
pub fn person_started_tx(conn: &Connection) -> Result<Vec<EffectRunKey>, DomainError> {
    let mut st = conn
        .prepare(
            "SELECT effect, event_id, event_seq, attempt, origin FROM effect_run
              WHERE state = 'started' AND origin <> 'live' ORDER BY id",
        )
        .map_err(map_sql_err)?;
    let rows: Vec<(String, String, i64, u32, String)> = st
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })
        .and_then(|rows| rows.collect::<rusqlite::Result<_>>())
        .map_err(map_sql_err)?;
    rows.into_iter()
        .map(|(effect, event_id, event_seq, attempt, origin)| {
            Ok(EffectRunKey {
                effect,
                event_id,
                event_seq,
                attempt,
                origin: ReactionOrigin::parse(&origin)?,
            })
        })
        .collect()
}

/// The state of `key`'s attempt, if it was made.
pub fn state_tx(conn: &Connection, key: &EffectRunKey) -> Result<Option<RunState>, DomainError> {
    let state: Option<String> = conn
        .query_row(
            "SELECT state FROM effect_run WHERE effect = ?1 AND event_id = ?2 AND attempt = ?3",
            params![key.effect, key.event_id, key.attempt],
            |r| r.get(0),
        )
        .optional()
        .map_err(map_sql_err)?;
    state.as_deref().map(RunState::parse).transpose()
}

fn already(key: &EffectRunKey) -> DomainError {
    DomainError::Invalid(format!(
        "effect `{}` already reacted to event {}",
        key.effect, key.event_id
    ))
}

/// Claim `key`'s attempt before a run with a step outside the
/// transaction: a `started` row, keeping `resend` — what it composed, when
/// every step is safe to send again — so an attempt cut off before it is
/// recorded can be sent again by itself (tsk954). `Invalid` when that
/// attempt was already made.
pub fn claim_tx(
    conn: &Connection,
    key: &EffectRunKey,
    now: &str,
    resend: Option<&str>,
) -> Result<(), DomainError> {
    let inserted = conn
        .execute(
            "INSERT INTO effect_run
                 (effect, event_id, event_seq, attempt, origin, state, started_at, resend_json)
             VALUES (?1, ?2, ?3, ?4, ?5, 'started', ?6, ?7)
             ON CONFLICT (effect, event_id, attempt) DO NOTHING",
            params![
                key.effect,
                key.event_id,
                key.event_seq,
                key.attempt,
                key.origin.as_str(),
                now,
                resend
            ],
        )
        .map_err(map_sql_err)?;
    if inserted == 0 {
        return Err(already(key));
    }
    Ok(())
}

/// A `started` attempt's claim: what it kept to send again, and when it
/// started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claimed {
    pub resend: Option<String>,
    pub started_at: String,
}

/// `key`'s claim, while its attempt is `started`.
pub fn claimed_tx(conn: &Connection, key: &EffectRunKey) -> Result<Option<Claimed>, DomainError> {
    conn.query_row(
        "SELECT resend_json, started_at FROM effect_run
          WHERE effect = ?1 AND event_id = ?2 AND attempt = ?3 AND state = 'started'",
        params![key.effect, key.event_id, key.attempt],
        |r| {
            Ok(Claimed {
                resend: r.get(0)?,
                started_at: r.get(1)?,
            })
        },
    )
    .optional()
    .map_err(map_sql_err)
}

/// Record how `key`'s attempt ended: its row, or its `started` claim
/// finished — keeping what the claim composed only when it failed, for its
/// automatic retry. `Invalid` when it already ended (a concurrent or
/// earlier delivery got there first).
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
                    SET state = ?4, reason = ?5, audit_id = ?6, proposal_id = ?7, finished_at = ?8,
                        resend_json = CASE WHEN ?4 = 'failed' THEN resend_json END
                  WHERE effect = ?1 AND event_id = ?2 AND attempt = ?3",
                params![
                    key.effect,
                    key.event_id,
                    key.attempt,
                    done.state.as_str(),
                    done.reason,
                    done.audit_id,
                    done.proposal_id,
                    now
                ],
            )
            .map_err(map_sql_err)?;
        }
        Some(_) => return Err(already(key)),
        None => {
            conn.execute(
                "INSERT INTO effect_run
                     (effect, event_id, event_seq, attempt, origin, state, reason, audit_id,
                      proposal_id, started_at, finished_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
                params![
                    key.effect,
                    key.event_id,
                    key.event_seq,
                    key.attempt,
                    key.origin.as_str(),
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
        EffectRunKey::first("acme/notify", "e1", 3, ReactionOrigin::Live)
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
            assert!(claim_tx(tx, &key(), "t", None).is_err());
            let other = EffectRunKey {
                event_id: "e2".into(),
                ..key()
            };
            claim_tx(tx, &other, "t", None)?;
            assert_eq!(state_tx(tx, &other)?, Some(RunState::Started));
            finish_tx(tx, &other, &ok(), "t")?;
            assert_eq!(state_tx(tx, &other)?, Some(RunState::Ok));
            Ok(())
        })
        .await
        .unwrap();
    }

    /// tsk954: an attempt claimed with what it composed keeps it while it
    /// runs — what is sent again when it is cut off — and after, only if
    /// it failed (for its automatic retry).
    #[tokio::test]
    async fn a_claim_keeps_what_it_composed() {
        let db = Database::in_memory();
        db.transaction(|tx| {
            claim_tx(tx, &key(), "t1", Some("{\"calls\":[]}"))?;
            assert_eq!(
                claimed_tx(tx, &key())?,
                Some(Claimed {
                    resend: Some("{\"calls\":[]}".into()),
                    started_at: "t1".into(),
                })
            );
            finish_tx(tx, &key(), &ok(), "t2")?;
            assert_eq!(claimed_tx(tx, &key())?, None, "no longer started");
            assert_eq!(latest_tx(tx, "acme/notify", "e1")?.unwrap().resend, None);
            let failing = EffectRunKey {
                event_id: "e2".into(),
                ..key()
            };
            claim_tx(tx, &failing, "t1", Some("x"))?;
            let failed = Finished {
                state: RunState::Failed,
                reason: Some("boom".into()),
                audit_id: None,
                proposal_id: None,
            };
            finish_tx(tx, &failing, &failed, "t2")?;
            assert_eq!(
                latest_tx(tx, "acme/notify", "e2")?
                    .unwrap()
                    .resend
                    .as_deref(),
                Some("x")
            );
            let plain = EffectRunKey {
                event_id: "e3".into(),
                ..key()
            };
            claim_tx(tx, &plain, "t1", None)?;
            assert_eq!(claimed_tx(tx, &plain)?.unwrap().resend, None);
            Ok(())
        })
        .await
        .unwrap();
    }

    /// P9.D4: a reaction's state is its latest attempt's. A retry is the
    /// next attempt — claimed and finished like the first — and an
    /// attempt is made once.
    #[tokio::test]
    async fn the_latest_attempt_is_the_reactions_state() {
        let db = Database::in_memory();
        db.transaction(|tx| {
            assert_eq!(latest_tx(tx, "acme/notify", "e1")?, None);
            let failed = Finished {
                state: RunState::Failed,
                reason: Some("interrupted".into()),
                audit_id: None,
                proposal_id: None,
            };
            finish_tx(tx, &key(), &failed, "t1")?;
            assert_eq!(
                latest_tx(tx, "acme/notify", "e1")?,
                Some(Latest {
                    attempt: 1,
                    state: RunState::Failed,
                    origin: ReactionOrigin::Live,
                    retry_at: None,
                    resend: None,
                })
            );
            let retry = EffectRunKey {
                attempt: 2,
                origin: ReactionOrigin::Retry,
                ..key()
            };
            claim_tx(tx, &retry, "t2", None)?;
            assert_eq!(
                latest_tx(tx, "acme/notify", "e1")?,
                Some(Latest {
                    attempt: 2,
                    state: RunState::Started,
                    origin: ReactionOrigin::Retry,
                    retry_at: None,
                    resend: None,
                })
            );
            assert!(
                claim_tx(tx, &retry, "t2", None).is_err(),
                "an attempt is made once"
            );
            finish_tx(tx, &retry, &ok(), "t3")?;
            assert_eq!(
                latest_tx(tx, "acme/notify", "e1")?.map(|l| (l.attempt, l.state)),
                Some((2, RunState::Ok))
            );
            // The first attempt stands as it ended.
            assert_eq!(state_tx(tx, &key())?, Some(RunState::Failed));
            let origins: Vec<(i64, String)> = tx
                .prepare("SELECT attempt, origin FROM effect_run ORDER BY attempt")
                .and_then(|mut st| {
                    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                        .collect::<rusqlite::Result<_>>()
                })
                .map_err(map_sql_err)?;
            assert_eq!(
                origins,
                vec![(1, "live".to_string()), (2, "retry".to_string())]
            );
            Ok(())
        })
        .await
        .unwrap();
    }
}
