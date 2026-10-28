//! How an agent's terminal ended, said to the person (`.context/terminal.md`
//! "Resume outcomes").
//!
//! A launch that resumes a saved conversation never falls back to a fresh
//! one. When its process exits, the pane is told what that meant: a
//! resume that exited nonzero **before the session recorded a turn** is
//! `unable_to_resume` — the saved conversation is kept, the person can
//! Retry or deliberately Start fresh. A nonzero exit after a turn ran is
//! an ordinary end (the agent crashed or was quit), as is a stop the
//! person asked for.

use oxplow_domain::{AgentSessionId, DomainError, Timestamp};
use rusqlite::{params, Connection};
use serde::Serialize;

/// How a session's process was launched.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LaunchInfo {
    /// It resumed a saved conversation.
    pub resumed: bool,
    /// The harness found that conversation's transcript missing (it
    /// resumed anyway; the harness has the final say).
    pub transcript_missing: bool,
}

/// What the database knows of a session that bears on how it ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExitFacts {
    /// The session recorded a turn at or after the launch.
    pub turn_since_launch: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UnableReason {
    /// The harness found no transcript for the saved conversation.
    TranscriptMissing,
    /// The resume exited before any turn ran.
    ExitedEarly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExitOutcome {
    /// An ordinary end: the agent quit, or crashed after a turn.
    Ended,
    /// The resume of a saved conversation failed before a turn ran.
    UnableToResume {
        reason: UnableReason,
        #[serde(rename = "exitCode")]
        exit_code: i32,
    },
}

/// How the process that exited with `exit_code` ended.
pub fn classify_exit(launch: LaunchInfo, exit_code: Option<i32>, facts: ExitFacts) -> ExitOutcome {
    let Some(code) = exit_code.filter(|c| *c != 0) else {
        return ExitOutcome::Ended;
    };
    if !launch.resumed || facts.turn_since_launch {
        return ExitOutcome::Ended;
    }
    ExitOutcome::UnableToResume {
        reason: if launch.transcript_missing {
            UnableReason::TranscriptMissing
        } else {
            UnableReason::ExitedEarly
        },
        exit_code: code,
    }
}

/// Whether `session` recorded a turn (a prompt) at or after `since`: the
/// one check that tells a resume that never got going from a session that
/// ran and later ended badly.
pub fn turn_since(
    conn: &Connection,
    session: AgentSessionId,
    since: Timestamp,
) -> Result<bool, DomainError> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM agent_turn WHERE agent_session_id = ?1 AND started_at >= ?2)",
        params![session.value(), oxplow_db::ts_to_string(since)],
        |r| r.get::<_, bool>(0),
    )
    .map_err(oxplow_db::map_sql_err)
}

/// The facts about `session` at its exit, given the launch time.
pub fn exit_facts(
    conn: &Connection,
    session: AgentSessionId,
    launched_at: Timestamp,
) -> Result<ExitFacts, DomainError> {
    Ok(ExitFacts {
        turn_since_launch: turn_since(conn, session, launched_at)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESUMED: LaunchInfo = LaunchInfo {
        resumed: true,
        transcript_missing: false,
    };

    fn facts(turn: bool) -> ExitFacts {
        ExitFacts {
            turn_since_launch: turn,
        }
    }

    #[test]
    fn a_resume_that_fails_before_a_turn_is_unable_to_resume() {
        assert_eq!(
            classify_exit(RESUMED, Some(1), facts(false)),
            ExitOutcome::UnableToResume {
                reason: UnableReason::ExitedEarly,
                exit_code: 1
            }
        );
        let missing = LaunchInfo {
            transcript_missing: true,
            ..RESUMED
        };
        assert_eq!(
            classify_exit(missing, Some(1), facts(false)),
            ExitOutcome::UnableToResume {
                reason: UnableReason::TranscriptMissing,
                exit_code: 1
            }
        );
    }

    #[test]
    fn every_other_end_is_an_ordinary_end() {
        // A nonzero exit after a turn ran.
        assert_eq!(
            classify_exit(RESUMED, Some(1), facts(true)),
            ExitOutcome::Ended
        );
        // A clean exit, or one with no code (a signal).
        assert_eq!(
            classify_exit(RESUMED, Some(0), facts(false)),
            ExitOutcome::Ended
        );
        assert_eq!(
            classify_exit(RESUMED, None, facts(false)),
            ExitOutcome::Ended
        );
        // It never resumed anything.
        assert_eq!(
            classify_exit(LaunchInfo::default(), Some(1), facts(false)),
            ExitOutcome::Ended
        );
    }

    /// A turn counts only when it began at or after the launch — both
    /// orderings.
    #[tokio::test]
    async fn a_turn_counts_only_from_the_launch_on() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let (session, thread) = (fx.session, fx.thread);
        let at = |ms: i64| Timestamp::from_unix_ms(1_700_000_000_000 + ms);
        let launched = at(10_000);
        let db = fx.svc.db.clone();
        let check = |db: oxplow_db::Database| async move {
            db.read(move |c| turn_since(c, session, launched))
                .await
                .unwrap()
        };
        assert!(!check(db.clone()).await, "no turn yet");
        let insert = |db: oxplow_db::Database, ms: i64| async move {
            db.transaction(move |tx| {
                tx.execute(
                    "INSERT INTO agent_turn (thread_id, agent_session_id, prompt, started_at)
                     VALUES (?1, ?2, 'go', ?3)",
                    params![
                        thread.value(),
                        session.value(),
                        oxplow_db::ts_to_string(at(ms))
                    ],
                )
                .map_err(oxplow_db::map_sql_err)?;
                Ok(())
            })
            .await
            .unwrap()
        };
        insert(db.clone(), 5_000).await; // before the launch
        assert!(
            !check(db.clone()).await,
            "a turn before the launch is not one since"
        );
        insert(db.clone(), 12_000).await; // after it
        assert!(check(db.clone()).await);
    }
}
