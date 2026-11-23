//! `effect_run`: each effect's
//! reaction to each event. The live consumer makes at most one attempt;
//! a person's retry of a failed reaction makes the next, so a reaction
//! is `(effect, event_id)` and its **state is its latest attempt's**.
//! The command bus writes an attempt's row in its run's own transaction.
//! A step outside that transaction first gets a claim with a NULL outcome.
//! The app's `effect_triggers` checks the claim on redelivery.

use rusqlite::{params, Connection, OptionalExtension};
use std::{sync::Arc, time::Duration};

use crate::database::map_sql_err;
use oxplow_domain::{
    run::{Outcome, RunContext},
    DomainError, Timestamp,
};

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

/// The latest attempt's shared outcome and its proposal, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunState {
    pub outcome: Option<Outcome>,
    pub proposal_id: Option<i64>,
}

impl RunState {
    /// Whether this attempt ended so that a retry may follow it: failed,
    /// without asking anyone. The attempt's part of `v_effect_run.retryable`
    /// (`the_dataset_and_retry_store_agree_on_which_attempts_can_retry`);
    /// the effect's part (declared and enabled now) is checked before a
    /// retry runs (`commands::effect::ready`).
    pub fn retryable(self) -> bool {
        self.proposal_id.is_none()
            && matches!(
                self.outcome,
                Some(
                    Outcome::Failed
                        | Outcome::Denied
                        | Outcome::Cancelled
                        | Outcome::Invalid
                        | Outcome::Interrupted
                )
            )
    }

    pub fn display(self) -> &'static str {
        if self.proposal_id.is_some() {
            "proposed"
        } else {
            self.outcome.map_or("started", Outcome::as_str)
        }
    }
}
/// How a reaction ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finished {
    pub outcome: Outcome,
    pub reason: Option<String>,
    pub command_run_id: Option<i64>,
    pub proposal_id: Option<i64>,
}

/// A candidate for an automatic retry of this completion.
pub struct AutomaticRetry {
    pub resend: String,
    pub asked: Option<Duration>,
    pub from: Timestamp,
}

const RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(10), Duration::from_secs(60)];
const MAX_ASKED_WAIT: Duration = Duration::from_secs(15 * 60);

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
            "SELECT a.attempt, r.outcome, a.proposal_id, a.origin, a.retry_at, a.resend_json
               FROM effect_run a JOIN contribution_run r USING (run_id)
              WHERE a.effect = ?1 AND a.event_id = ?2 ORDER BY a.attempt DESC LIMIT 1",
            params![effect, event_id],
            |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get(2)?,
                    r.get::<_, String>(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()
        .map_err(map_sql_err)?;
    latest
        .map(
            |(attempt, outcome, proposal_id, origin, retry_at, resend)| {
                Ok(Latest {
                    attempt,
                    state: RunState {
                        outcome: outcome.as_deref().map(str::parse).transpose()?,
                        proposal_id,
                    },
                    origin: ReactionOrigin::parse(&origin)?,
                    retry_at,
                    resend,
                })
            },
        )
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
/// automatic attempt sends `resend`, what the failed one composed.
pub fn schedule_retry_tx(
    conn: &Connection,
    key: &EffectRunKey,
    at: &str,
    resend: &str,
) -> Result<(), DomainError> {
    conn.execute(
        "UPDATE effect_run SET retry_at = ?4, resend_json = ?5
          WHERE effect = ?1 AND event_id = ?2 AND attempt = ?3
            AND proposal_id IS NULL
            AND run_id IN (SELECT run_id FROM contribution_run WHERE outcome IN ('failed', 'interrupted'))",
        params![key.effect, key.event_id, key.attempt, at, resend],
    )
    .map_err(map_sql_err)?;
    Ok(())
}

/// Drop `key`'s scheduled retry: it won't be sent again by itself.
pub(crate) fn drop_retry_tx(
    conn: &Connection,
    key: &EffectRunKey,
    health: Option<(&oxplow_domain::vocabulary::Vocabulary, &str)>,
) -> Result<(), DomainError> {
    let dropped = conn
        .execute(
            "UPDATE effect_run SET retry_at = NULL, resend_json = NULL
          WHERE effect = ?1 AND event_id = ?2 AND attempt = ?3 AND retry_at IS NOT NULL",
            params![key.effect, key.event_id, key.attempt],
        )
        .map_err(map_sql_err)?;
    if dropped > 0 {
        if let Some((vocabulary, reason)) = health {
            account_completion_tx(
                conn,
                vocabulary,
                key,
                &Finished {
                    outcome: Outcome::Failed,
                    reason: Some(reason.into()),
                    command_run_id: None,
                    proposal_id: None,
                },
                None,
            )?;
        }
    }
    Ok(())
}

/// A retry that cannot run becomes a person's to retry. Clear its schedule
/// and count the failure together when the effect still exists.
pub async fn abandon_retry(
    db: &crate::Database,
    key: EffectRunKey,
    health: Option<(Arc<oxplow_domain::vocabulary::Vocabulary>, String)>,
) -> Result<(), DomainError> {
    db.transaction(move |tx| {
        drop_retry_tx(
            tx,
            &key,
            health
                .as_ref()
                .map(|(vocabulary, reason)| (vocabulary.as_ref(), reason.as_str())),
        )
    })
    .await
}

/// Drop every scheduled retry of `effect`: what they would send
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
               FROM effect_run r JOIN contribution_run c USING (run_id)
              WHERE c.outcome IN ('failed', 'interrupted') AND r.proposal_id IS NULL AND r.retry_at IS NOT NULL AND r.retry_at <= ?1
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

/// Attempts whose claim survived a stop. At boot the host decides which
/// live attempts can be recovered by redelivery.
pub fn unfinished_tx(conn: &Connection) -> Result<Vec<EffectRunKey>, DomainError> {
    let mut st = conn
        .prepare(
            "SELECT a.effect, a.event_id, a.event_seq, a.attempt, a.origin
               FROM effect_run a JOIN contribution_run r USING (run_id)
              WHERE r.outcome IS NULL ORDER BY a.id",
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

/// Every reaction whose latest attempt can be retried, as `(effect,
/// event_id)` in the order the attempts were recorded: what a person's
/// Retry all runs again. `v_effect_run.retryable` is the one rule (the
/// attempt failed, and the effect is declared and enabled now), so Alerts
/// and Retry all list the same reactions.
pub fn failed_latest_tx(conn: &Connection) -> Result<Vec<(String, String)>, DomainError> {
    let mut st = conn
        .prepare(
            "SELECT effect, event_id FROM v_effect_run
              WHERE latest = 1 AND retryable = 1 ORDER BY id",
        )
        .map_err(map_sql_err)?;
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .and_then(|rows| rows.collect::<rusqlite::Result<_>>())
        .map_err(map_sql_err)
}

/// The claimed shared run and its outcome, if this attempt was made.
fn attempt_tx(
    conn: &Connection,
    key: &EffectRunKey,
) -> Result<Option<(String, RunState)>, DomainError> {
    let found = conn
        .query_row(
            "SELECT r.run_id, r.outcome, a.proposal_id
           FROM effect_run a JOIN contribution_run r USING (run_id)
          WHERE a.effect = ?1 AND a.event_id = ?2 AND a.attempt = ?3",
            params![key.effect, key.event_id, key.attempt],
            |r| Ok((r.get(0)?, r.get::<_, Option<String>>(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(map_sql_err)?;
    found
        .map(|(id, outcome, proposal_id)| {
            Ok((
                id,
                RunState {
                    outcome: outcome.as_deref().map(str::parse).transpose()?,
                    proposal_id,
                },
            ))
        })
        .transpose()
}

#[cfg(test)]
fn state_tx(conn: &Connection, key: &EffectRunKey) -> Result<Option<RunState>, DomainError> {
    Ok(attempt_tx(conn, key)?.map(|(_, state)| state))
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
/// recorded can be sent again by itself. `Invalid` when that
/// attempt was already made.
pub fn claim_tx(
    conn: &Connection,
    key: &EffectRunKey,
    run: &RunContext,
    resend: Option<&str>,
) -> Result<(), DomainError> {
    if attempt_tx(conn, key)?.is_some() {
        return Err(already(key));
    }
    let valid = run.contribution_ref().is_some_and(|reference| {
        reference.id.kind == oxplow_domain::contribution::ContributionKind::Effect
            && key.effect == format!("{}/{}", reference.id.namespace, reference.id.local)
    });
    if !valid {
        return Err(DomainError::Invariant(format!(
            "attempt for {} needs its own effect run",
            key.effect,
        )));
    }
    crate::contribution_run_store::begin_tx(conn, run)?;
    conn.execute(
        "INSERT INTO effect_run (effect, event_id, event_seq, attempt, origin, resend_json, run_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            key.effect,
            key.event_id,
            key.event_seq,
            key.attempt,
            key.origin.as_str(),
            resend,
            run.run().as_str()
        ],
    )
    .map_err(map_sql_err)?;
    // The attempts before it keep no composition. Their retry time records
    // that it was sent again.
    conn.execute(
        "UPDATE effect_run SET resend_json = NULL
          WHERE effect = ?1 AND event_id = ?2 AND attempt < ?3 AND resend_json IS NOT NULL",
        params![key.effect, key.event_id, key.attempt],
    )
    .map_err(map_sql_err)?;
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
        "SELECT a.resend_json, r.started_at FROM effect_run a JOIN contribution_run r USING (run_id)
          WHERE a.effect = ?1 AND a.event_id = ?2 AND a.attempt = ?3 AND r.outcome IS NULL",
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
/// finished — dropping what the claim composed: only a scheduled retry
/// keeps a composition ([`schedule_retry_tx`], in the same transaction
/// when the failure is sent again), so none outlives the retry it was for.
/// `Invalid` when it already ended (a concurrent or earlier
/// delivery got there first).
pub fn finish_tx(
    conn: &Connection,
    key: &EffectRunKey,
    done: &Finished,
    run: Option<&RunContext>,
    health: Option<&oxplow_domain::vocabulary::Vocabulary>,
) -> Result<(), DomainError> {
    let run_id = match (attempt_tx(conn, key)?, run) {
        (Some((id, state)), Some(run)) if state.outcome.is_none() && id == run.run().as_str() => {
            crate::contribution_run_store::finish_tx(
                conn,
                run,
                done.outcome,
                done.reason.as_deref(),
            )?;
            id
        }
        (None, Some(run)) => {
            claim_tx(conn, key, run, None)?;
            crate::contribution_run_store::finish_tx(
                conn,
                run,
                done.outcome,
                done.reason.as_deref(),
            )?;
            run.run().to_string()
        }
        (Some((id, state)), None)
            if state.outcome.is_none() && done.outcome == Outcome::Interrupted =>
        {
            conn.execute(
                "UPDATE contribution_run
                    SET finished_at = ?2, outcome = 'interrupted', error = ?3
                  WHERE run_id = ?1 AND outcome IS NULL",
                params![id, Timestamp::now().to_string(), done.reason],
            )
            .map_err(map_sql_err)?;
            id
        }
        _ => return Err(already(key)),
    };
    conn.execute(
        "UPDATE effect_run SET command_run_id = ?2, proposal_id = ?3, resend_json = NULL WHERE run_id = ?1",
        params![run_id, done.command_run_id, done.proposal_id],
    ).map_err(map_sql_err)?;
    if let Some(vocabulary) = health {
        account_completion_tx(conn, vocabulary, key, done, run)?;
    }
    Ok(())
}

/// Finish an attempt and either schedule its retry or account for its
/// terminal outcome. The caller commits them with its own run and events.
pub fn finish_with_retry_tx(
    conn: &Connection,
    key: &EffectRunKey,
    done: &Finished,
    run: Option<&RunContext>,
    health: Option<&oxplow_domain::vocabulary::Vocabulary>,
    retry: Option<&AutomaticRetry>,
) -> Result<bool, DomainError> {
    let retry = retry.filter(|retry| {
        done.proposal_id.is_none()
            && matches!(done.outcome, Outcome::Failed | Outcome::Interrupted)
            && retry.asked.is_none_or(|asked| asked <= MAX_ASKED_WAIT)
    });
    let scheduled = if let Some(retry) = retry {
        let made = automatic_in_a_row_tx(conn, &key.effect, &key.event_id)?
            + u32::from(key.origin == ReactionOrigin::Auto && attempt_tx(conn, key)?.is_none());
        RETRY_DELAYS.get(made as usize).map(|&delay| {
            let delay = retry.asked.map_or(delay, |asked| delay.max(asked));
            let at = Timestamp::from_unix_ms(retry.from.unix_ms() + delay.as_millis() as i64);
            (at, retry.resend.as_str())
        })
    } else {
        None
    };
    finish_tx(conn, key, done, run, health.filter(|_| scheduled.is_none()))?;
    if let Some((at, resend)) = scheduled {
        schedule_retry_tx(conn, key, &at.to_string(), resend)?;
        return Ok(true);
    }
    Ok(false)
}

/// Account for a recorded outcome once retry policy decides it is final.
pub(crate) fn account_completion_tx(
    conn: &Connection,
    vocabulary: &oxplow_domain::vocabulary::Vocabulary,
    key: &EffectRunKey,
    done: &Finished,
    run: Option<&RunContext>,
) -> Result<(), DomainError> {
    if let Some(run) = run {
        if !attempt_tx(conn, key)?.is_some_and(|(id, _)| id == run.run().as_str()) {
            return Ok(());
        }
    }
    let (namespace, id) = key.effect.split_once('/').unwrap_or(("", &key.effect));
    let health_key = crate::ContributionKey {
        namespace: namespace.to_string(),
        contribution: id.to_string(),
        kind: "effect",
    };
    crate::contribution_health_store::record_completion_tx(
        conn,
        vocabulary,
        &health_key,
        done.outcome,
        done.reason.as_deref(),
        run.map(|run| run.elapsed().as_secs_f64() * 1000.0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    fn key() -> EffectRunKey {
        EffectRunKey::first("acme/notify", "e1", 3, ReactionOrigin::Live)
    }

    pub(crate) fn context(key: &EffectRunKey) -> RunContext {
        use oxplow_domain::contribution::{ContributionId, ContributionKind};
        use oxplow_domain::run::{ContributionRef, Mode};
        let (namespace, local) = key.effect.split_once('/').unwrap();
        RunContext::contribution(
            ContributionRef {
                id: ContributionId::new(namespace, ContributionKind::Effect, local),
                extension: namespace.into(),
                version: None,
                generation: None,
                deployment: None,
            },
            oxplow_domain::Actor::System,
            Mode::External,
        )
        .caused_by(oxplow_domain::EventId(key.event_id.clone()))
    }

    fn state(outcome: Option<Outcome>) -> RunState {
        RunState {
            outcome,
            proposal_id: None,
        }
    }

    fn ok() -> Finished {
        Finished {
            outcome: Outcome::Ok,
            reason: None,
            command_run_id: Some(9),
            proposal_id: None,
        }
    }

    #[tokio::test]
    async fn a_reaction_ends_once_whether_claimed_first_or_not() {
        let db = Database::in_memory();
        db.transaction(|tx| {
            finish_tx(tx, &key(), &ok(), Some(&context(&key())), None)?;
            assert_eq!(state_tx(tx, &key())?, Some(state(Some(Outcome::Ok))));
            assert!(finish_tx(tx, &key(), &ok(), Some(&context(&key())), None).is_err());
            assert!(claim_tx(tx, &key(), &context(&key()), None).is_err());
            let other = EffectRunKey {
                event_id: "e2".into(),
                ..key()
            };
            let run = context(&other);
            claim_tx(tx, &other, &run, None)?;
            assert_eq!(state_tx(tx, &other)?, Some(state(None)));
            finish_tx(tx, &other, &ok(), Some(&run), None)?;
            assert_eq!(state_tx(tx, &other)?, Some(state(Some(Outcome::Ok))));
            assert_eq!(
                crate::event_retention::sweep_run_batch_tx(tx, "2099-02-01", "2099-01-01", 100)?,
                (0, 0)
            );
            assert_eq!(state_tx(tx, &key())?, Some(state(Some(Outcome::Ok))));
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn an_effect_finish_rolls_back_when_its_health_write_fails() {
        let db = Database::in_memory();
        db.call(|conn| {
            conn.execute_batch(
                "CREATE TRIGGER reject_health BEFORE INSERT ON contribution_health
             BEGIN SELECT RAISE(ABORT, 'health write failed'); END;",
            )
        })
        .await
        .unwrap();
        let result = db
            .transaction(|tx| {
                let key = key();
                let run = context(&key);
                let vocabulary = oxplow_domain::vocabulary::Vocabulary::core();
                finish_tx(tx, &key, &ok(), Some(&run), Some(&vocabulary))
            })
            .await;
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("health write failed"));
        let counts = db
            .call(|conn| {
                conn.query_row(
            "SELECT (SELECT count(*) FROM effect_run), (SELECT count(*) FROM contribution_run)",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
            })
            .await
            .unwrap();
        assert_eq!(counts, (0, 0));
    }

    /// an attempt claimed with what it composed keeps it while it
    /// runs — what is sent again when it is cut off. finished, it
    /// drops it; only a scheduled retry keeps a composition, so none
    /// outlives the retry it was for.
    #[tokio::test]
    async fn a_resendable_failure_without_a_retry_finishes_with_its_health_count() {
        let db = Database::in_memory();
        db.transaction(|tx| {
            let first = key();
            let run = context(&first);
            let failed = Finished {
                outcome: Outcome::Failed,
                reason: Some("step unavailable".into()),
                command_run_id: None,
                proposal_id: None,
            };
            let vocabulary = oxplow_domain::vocabulary::Vocabulary::core();
            let mut retry = AutomaticRetry {
                resend: "{}".into(),
                asked: None,
                from: Timestamp::now(),
            };
            claim_tx(tx, &first, &run, Some(&retry.resend))?;
            assert!(finish_with_retry_tx(
                tx,
                &first,
                &failed,
                Some(&run),
                Some(&vocabulary),
                Some(&retry)
            )?);
            let failures = |tx: &Connection| {
                tx.query_row(
                    "SELECT consecutive_failures FROM contribution_health
                      WHERE namespace = 'acme' AND contribution = 'notify' AND kind = 'effect'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .optional()
                .map_err(map_sql_err)
            };
            assert_eq!(failures(tx)?, None);

            retry.asked = Some(MAX_ASKED_WAIT + Duration::from_secs(1));
            let automatic = EffectRunKey {
                attempt: 2,
                origin: ReactionOrigin::Auto,
                ..first
            };
            let run = context(&automatic);
            claim_tx(tx, &automatic, &run, Some(&retry.resend))?;
            assert!(!finish_with_retry_tx(
                tx,
                &automatic,
                &failed,
                Some(&run),
                Some(&vocabulary),
                Some(&retry)
            )?);
            assert_eq!(
                state_tx(tx, &automatic)?,
                Some(state(Some(Outcome::Failed)))
            );
            assert_eq!(failures(tx)?, Some(1));
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn a_claim_keeps_what_it_composed() {
        let db = Database::in_memory();
        db.transaction(|tx| {
            let run = context(&key());
            claim_tx(tx, &key(), &run, Some("{\"calls\":[]}"))?;
            assert_eq!(
                claimed_tx(tx, &key())?,
                Some(Claimed {
                    resend: Some("{\"calls\":[]}".into()),
                    started_at: run.began().to_string(),
                })
            );
            finish_tx(tx, &key(), &ok(), Some(&run), None)?;
            assert_eq!(claimed_tx(tx, &key())?, None, "no longer started");
            assert_eq!(latest_tx(tx, "acme/notify", "e1")?.unwrap().resend, None);
            let failing = EffectRunKey {
                event_id: "e2".into(),
                ..key()
            };
            let run = context(&failing);
            claim_tx(tx, &failing, &run, Some("x"))?;
            let failed = Finished {
                outcome: Outcome::Failed,
                reason: Some("boom".into()),
                command_run_id: None,
                proposal_id: None,
            };
            finish_tx(tx, &failing, &failed, Some(&run), None)?;
            assert_eq!(latest_tx(tx, "acme/notify", "e2")?.unwrap().resend, None);
            schedule_retry_tx(tx, &failing, "t3", "x")?;
            assert_eq!(
                latest_tx(tx, "acme/notify", "e2")?
                    .unwrap()
                    .resend
                    .as_deref(),
                Some("x")
            );
            // Its retry claimed: the composition moves to the attempt
            // sending it, and the failed one keeps none.
            let retrying = EffectRunKey {
                attempt: 2,
                origin: ReactionOrigin::Auto,
                ..failing.clone()
            };
            claim_tx(tx, &retrying, &context(&retrying), Some("x"))?;
            let kept: Vec<Option<String>> = tx
                .prepare(
                    "SELECT resend_json FROM effect_run WHERE event_id = 'e2' ORDER BY attempt",
                )
                .map_err(map_sql_err)?
                .query_map([], |r| r.get(0))
                .map_err(map_sql_err)?
                .collect::<Result<_, _>>()
                .map_err(map_sql_err)?;
            assert_eq!(kept, vec![None, Some("x".into())]);
            let plain = EffectRunKey {
                event_id: "e3".into(),
                ..key()
            };
            claim_tx(tx, &plain, &context(&plain), None)?;
            assert_eq!(claimed_tx(tx, &plain)?.unwrap().resend, None);
            Ok(())
        })
        .await
        .unwrap();
    }

    /// `acme/notify` (`namespace/local`) declared, as the catalog restates
    /// an enabled extension's effects into `contribution`.
    fn declare(tx: &Connection, namespace: &str, local: &str) -> Result<(), DomainError> {
        tx.execute(
            "INSERT INTO contribution (id, extension, kind, local, origin, runs_on_json,
                                       needs_json, after_json, file, namespace)
             VALUES (?1, ?2, 'effect', ?3, 'project', '{}', '[]', '[]', 'extension.yaml', ?2)",
            params![format!("{namespace}/effect/{local}"), namespace, local],
        )
        .map_err(map_sql_err)?;
        Ok(())
    }

    /// Retryable is the effect's now: a failed attempt of an effect no
    /// enabled extension declares (its extension turned off), or of one
    /// disabled, can't be retried (`ready` refuses), so the dataset (and
    /// Alerts, which reads it) and Retry all leave it out.
    #[tokio::test]
    async fn a_failed_attempt_of_an_undeclared_or_disabled_effect_is_not_retryable() {
        let db = Database::in_memory();
        db.transaction(|tx| {
            declare(tx, "acme", "notify")?;
            declare(tx, "acme", "off")?;
            tx.execute(
                "INSERT INTO contribution_health (namespace, contribution, kind, state, updated_at)
                 VALUES ('acme', 'off', 'effect', 'disabled', '2026-01-01T00:00:00Z')",
                [],
            )
            .map_err(map_sql_err)?;
            for effect in ["acme/notify", "acme/off", "gone/notify"] {
                let key = EffectRunKey::first(effect, "e1", 3, ReactionOrigin::Live);
                let failed = Finished {
                    outcome: Outcome::Cancelled,
                    reason: Some("its extension was turned off".into()),
                    command_run_id: None,
                    proposal_id: None,
                };
                finish_tx(tx, &key, &failed, Some(&context(&key)), None)?;
            }
            let retryable: Vec<String> = tx
                .prepare("SELECT effect FROM v_effect_run WHERE retryable = 1 ORDER BY effect")
                .and_then(|mut st| {
                    st.query_map([], |row| row.get(0))?
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .map_err(map_sql_err)?;
            assert_eq!(retryable, vec!["acme/notify".to_string()]);
            assert_eq!(
                failed_latest_tx(tx)?,
                vec![("acme/notify".to_string(), "e1".to_string())]
            );
            Ok(())
        })
        .await
        .unwrap();
    }

    /// P9.D4: a reaction's state is its latest attempt's. A retry is the
    /// next attempt — claimed and finished like the first — and an
    /// attempt is made once.
    #[tokio::test]
    async fn the_dataset_and_retry_store_agree_on_which_attempts_can_retry() {
        let db = Database::in_memory();
        db.transaction(|tx| {
            declare(tx, "acme", "notify")?;
            let outcomes = [
                None,
                Some(Outcome::Ok),
                Some(Outcome::Skipped),
                Some(Outcome::Failed),
                Some(Outcome::Denied),
                Some(Outcome::Cancelled),
                Some(Outcome::Invalid),
                Some(Outcome::Interrupted),
            ];
            for (index, outcome) in outcomes.into_iter().enumerate() {
                for proposal_id in [None, Some(1)] {
                    let key = EffectRunKey {
                        event_id: format!("event-{index}-{proposal_id:?}"),
                        ..key()
                    };
                    let run = context(&key);
                    let state = RunState { outcome, proposal_id };
                    if let Some(outcome) = outcome {
                        finish_tx(
                            tx,
                            &key,
                            &Finished {
                                outcome,
                                reason: None,
                                command_run_id: None,
                                proposal_id,
                            },
                            Some(&run),
                            None,
                        )?;
                    } else {
                        claim_tx(tx, &key, &run, None)?;
                    }
                    let retryable: bool = tx
                        .query_row(
                            "SELECT retryable FROM v_effect_run WHERE event_id = ?1",
                            [&key.event_id],
                            |row| row.get(0),
                        )
                        .map_err(map_sql_err)?;
                    assert_eq!(retryable, state.retryable(), "{state:?}");
                }
            }
            let listed = tx
                .prepare("SELECT effect, event_id FROM v_effect_run WHERE latest = 1 AND retryable = 1 ORDER BY id")
                .and_then(|mut statement| {
                    statement
                        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
                        .collect::<rusqlite::Result<Vec<_>>>()
                })
                .map_err(map_sql_err)?;
            assert_eq!(listed, failed_latest_tx(tx)?);
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn the_latest_attempt_is_the_reactions_state() {
        let db = Database::in_memory();
        db.transaction(|tx| {
            assert_eq!(latest_tx(tx, "acme/notify", "e1")?, None);
            let failed = Finished {
                outcome: Outcome::Failed,
                reason: Some("interrupted".into()),
                command_run_id: None,
                proposal_id: None,
            };
            finish_tx(tx, &key(), &failed, Some(&context(&key())), None)?;
            assert_eq!(
                latest_tx(tx, "acme/notify", "e1")?,
                Some(Latest {
                    attempt: 1,
                    state: state(Some(Outcome::Failed)),
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
            let run = context(&retry);
            claim_tx(tx, &retry, &run, None)?;
            assert_eq!(
                latest_tx(tx, "acme/notify", "e1")?,
                Some(Latest {
                    attempt: 2,
                    state: state(None),
                    origin: ReactionOrigin::Retry,
                    retry_at: None,
                    resend: None,
                })
            );
            assert!(
                claim_tx(tx, &retry, &context(&retry), None).is_err(),
                "an attempt is made once"
            );
            finish_tx(tx, &retry, &ok(), Some(&run), None)?;
            assert_eq!(
                latest_tx(tx, "acme/notify", "e1")?.map(|l| (l.attempt, l.state)),
                Some((2, state(Some(Outcome::Ok))))
            );
            // The first attempt stands as it ended.
            assert_eq!(state_tx(tx, &key())?, Some(state(Some(Outcome::Failed))));
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
