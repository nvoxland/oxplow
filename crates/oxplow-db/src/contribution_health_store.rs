//! `contribution_health` (V135, P7.C1): each extension contribution's health on this
//! machine — a provider instance, a collector. The policy (when a failure
//! disables, what gets logged) is the app's `contribution_health.rs`; these are
//! its row updates, each runnable on a caller's transaction so a
//! transition commits with its event.

use rusqlite::{named_params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::database::map_sql_err;
use crate::Database;
use oxplow_domain::DomainError;

/// Which contribution: its `extension`, its `kind` (`provider`,
/// `collector`) and `contribution` (that provider's or collector's id) —
/// all three: a provider and a collector may share an id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContributionKey {
    pub extension: String,
    pub contribution: String,
    pub kind: &'static str,
}

/// One `contribution_health` row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ContributionHealthRow {
    pub extension: String,
    pub contribution: String,
    pub kind: String,
    /// `ok`, `failing` or `disabled`.
    pub state: String,
    pub reason: Option<String>,
    pub consecutive_failures: i64,
    pub last_ok_at: Option<String>,
    pub last_error: Option<String>,
    pub mean_ms: Option<f64>,
    pub next_due_at: Option<String>,
    pub updated_at: String,
    /// The repair work item filed when it was last disabled.
    pub repair_item: Option<String>,
    /// The last `contribution.disabled` the repair consumer handled.
    pub repair_seq: Option<i64>,
}

const SELECT: &str = "SELECT extension, contribution, kind, state, reason, consecutive_failures,
        last_ok_at, last_error, mean_ms, next_due_at, updated_at, repair_item, repair_seq
        FROM contribution_health";

/// The row a [`ContributionKey`] names, its parameters `:extension`, `:kind`,
/// `:contribution`.
const KEY: &str = "extension = :extension AND kind = :kind AND contribution = :contribution";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<ContributionHealthRow> {
    Ok(ContributionHealthRow {
        extension: r.get(0)?,
        contribution: r.get(1)?,
        kind: r.get(2)?,
        state: r.get(3)?,
        reason: r.get(4)?,
        consecutive_failures: r.get(5)?,
        last_ok_at: r.get(6)?,
        last_error: r.get(7)?,
        mean_ms: r.get(8)?,
        next_due_at: r.get(9)?,
        updated_at: r.get(10)?,
        repair_item: r.get(11)?,
        repair_seq: r.get(12)?,
    })
}

/// The row for `key`, if it has one.
pub fn get_tx(
    c: &Connection,
    key: &ContributionKey,
) -> Result<Option<ContributionHealthRow>, DomainError> {
    c.query_row(
        &format!("{SELECT} WHERE {KEY}"),
        named_params! { ":extension": key.extension, ":kind": key.kind, ":contribution": key.contribution },
        row,
    )
    .optional()
    .map_err(map_sql_err)
}

/// Make sure `key` has a row (state `ok`).
fn ensure(c: &Connection, key: &ContributionKey, now: &str) -> Result<(), DomainError> {
    c.execute(
        "INSERT INTO contribution_health (extension, contribution, kind, state, updated_at)
         VALUES (:extension, :contribution, :kind, 'ok', :now)
         ON CONFLICT (extension, kind, contribution) DO NOTHING",
        named_params! {
            ":extension": key.extension, ":kind": key.kind, ":contribution": key.contribution,
            ":now": now,
        },
    )
    .map(|_| ())
    .map_err(map_sql_err)
}

/// Count a failure; returns the failures in a row now. A disabled
/// contribution stays disabled.
pub fn failed_tx(
    c: &Connection,
    key: &ContributionKey,
    error: &str,
    now: &str,
) -> Result<i64, DomainError> {
    ensure(c, key, now)?;
    c.query_row(
        &format!(
            "UPDATE contribution_health SET
               consecutive_failures = consecutive_failures + 1,
               last_error = :error,
               state = CASE WHEN state = 'disabled' THEN 'disabled' ELSE 'failing' END,
               updated_at = :now
             WHERE {KEY}
             RETURNING consecutive_failures"
        ),
        named_params! {
            ":extension": key.extension, ":kind": key.kind, ":contribution": key.contribution,
            ":error": error, ":now": now,
        },
        |r| r.get(0),
    )
    .map_err(map_sql_err)
}

/// A success: the failure count starts over, and `took_ms` (a timed run
/// or call; `None` for one that wasn't, like a start) joins the moving
/// average.
pub fn succeeded_tx(
    c: &Connection,
    key: &ContributionKey,
    took_ms: Option<f64>,
    now: &str,
) -> Result<(), DomainError> {
    ensure(c, key, now)?;
    c.execute(
        &format!(
            "UPDATE contribution_health SET
               consecutive_failures = 0,
               state = CASE WHEN state = 'disabled' THEN 'disabled' ELSE 'ok' END,
               last_ok_at = :now,
               mean_ms = CASE WHEN :took IS NULL THEN mean_ms
                              WHEN mean_ms IS NULL THEN :took
                              ELSE mean_ms + (:took - mean_ms) / 10.0 END,
               updated_at = :now
             WHERE {KEY}"
        ),
        named_params! {
            ":extension": key.extension, ":kind": key.kind, ":contribution": key.contribution,
            ":took": took_ms, ":now": now,
        },
    )
    .map(|_| ())
    .map_err(map_sql_err)
}

/// Disable it, saying why.
pub fn disable_tx(
    c: &Connection,
    key: &ContributionKey,
    reason: &str,
    now: &str,
) -> Result<(), DomainError> {
    ensure(c, key, now)?;
    c.execute(
        &format!(
            "UPDATE contribution_health SET state = 'disabled', reason = :reason, updated_at = :now
             WHERE {KEY}"
        ),
        named_params! {
            ":extension": key.extension, ":kind": key.kind, ":contribution": key.contribution,
            ":reason": reason, ":now": now,
        },
    )
    .map(|_| ())
    .map_err(map_sql_err)
}

/// A person enabled it again: `ok`, its failure count starting over.
pub fn enable_tx(c: &Connection, key: &ContributionKey, now: &str) -> Result<(), DomainError> {
    ensure(c, key, now)?;
    c.execute(
        &format!(
            "UPDATE contribution_health SET state = 'ok', reason = NULL, consecutive_failures = 0,
               updated_at = :now
             WHERE {KEY}"
        ),
        named_params! {
            ":extension": key.extension, ":kind": key.kind, ":contribution": key.contribution,
            ":now": now,
        },
    )
    .map(|_| ())
    .map_err(map_sql_err)
}

/// When it should have run again by (`None`: it doesn't run on a
/// schedule — then a contribution without a row gets none).
pub fn set_next_due_tx(
    c: &Connection,
    key: &ContributionKey,
    next_due_at: Option<&str>,
    now: &str,
) -> Result<(), DomainError> {
    if next_due_at.is_some() {
        ensure(c, key, now)?;
    }
    c.execute(
        &format!("UPDATE contribution_health SET next_due_at = :due WHERE {KEY}"),
        named_params! {
            ":extension": key.extension, ":kind": key.kind, ":contribution": key.contribution,
            ":due": next_due_at,
        },
    )
    .map(|_| ())
    .map_err(map_sql_err)
}

/// Record that the `contribution.disabled` at `seq` was handled, filing or
/// commenting on `repair_item`.
pub fn set_repair_tx(
    c: &Connection,
    key: &ContributionKey,
    repair_item: &str,
    seq: i64,
) -> Result<(), DomainError> {
    c.execute(
        &format!(
            "UPDATE contribution_health SET repair_item = :item, repair_seq = :seq WHERE {KEY}"
        ),
        named_params! {
            ":extension": key.extension, ":kind": key.kind, ":contribution": key.contribution,
            ":item": repair_item, ":seq": seq,
        },
    )
    .map(|_| ())
    .map_err(map_sql_err)
}

#[derive(Clone)]
pub struct SqliteContributionHealthStore {
    db: Database,
}

impl SqliteContributionHealthStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    pub async fn get(
        &self,
        key: &ContributionKey,
    ) -> Result<Option<ContributionHealthRow>, DomainError> {
        let key = key.clone();
        self.db.read(move |c| get_tx(c, &key)).await
    }

    /// Forget `key`'s row: its contribution is gone.
    pub async fn remove(&self, key: &ContributionKey) -> Result<(), DomainError> {
        let key = key.clone();
        self.db
            .transaction(move |c| {
                c.execute(
                    &format!("DELETE FROM contribution_health WHERE {KEY}"),
                    named_params! { ":extension": key.extension, ":kind": key.kind, ":contribution": key.contribution },
                )
                .map_err(map_sql_err)?;
                Ok(())
            })
            .await
    }

    pub async fn list(&self) -> Result<Vec<ContributionHealthRow>, DomainError> {
        self.db
            .read(|c| {
                let mut st = c
                    .prepare(&format!("{SELECT} ORDER BY extension, contribution"))
                    .map_err(map_sql_err)?;
                let rows = st.query_map([], row).map_err(map_sql_err)?;
                rows.collect::<rusqlite::Result<_>>().map_err(map_sql_err)
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> ContributionKey {
        ContributionKey {
            extension: "tracker".into(),
            contribution: "fake".into(),
            kind: "provider",
        }
    }

    /// Failures count up and a success starts them over; a disable sticks
    /// through both until an enable.
    #[tokio::test]
    async fn failures_count_and_a_disable_sticks_until_enabled() {
        let db = Database::in_memory();
        let store = SqliteContributionHealthStore::new(db.clone());
        let k = key();
        let n = db
            .transaction({
                let k = k.clone();
                move |tx| {
                    failed_tx(tx, &k, "a", "t1")?;
                    failed_tx(tx, &k, "b", "t2")
                }
            })
            .await
            .unwrap();
        assert_eq!(n, 2);
        let r = store.get(&k).await.unwrap().unwrap();
        assert_eq!(
            (r.state.as_str(), r.last_error.as_deref()),
            ("failing", Some("b"))
        );
        db.transaction({
            let k = k.clone();
            move |tx| succeeded_tx(tx, &k, Some(40.0), "t3")
        })
        .await
        .unwrap();
        let r = store.get(&k).await.unwrap().unwrap();
        assert_eq!(
            (r.state.as_str(), r.consecutive_failures, r.mean_ms),
            ("ok", 0, Some(40.0))
        );

        db.transaction({
            let k = k.clone();
            move |tx| {
                disable_tx(tx, &k, "broken", "t4")?;
                failed_tx(tx, &k, "c", "t5")?;
                succeeded_tx(tx, &k, None, "t6")
            }
        })
        .await
        .unwrap();
        let r = store.get(&k).await.unwrap().unwrap();
        assert_eq!(
            (r.state.as_str(), r.reason.as_deref()),
            ("disabled", Some("broken"))
        );
        db.transaction({
            let k = k.clone();
            move |tx| enable_tx(tx, &k, "t7")
        })
        .await
        .unwrap();
        let r = store.get(&k).await.unwrap().unwrap();
        assert_eq!(
            (r.state.as_str(), r.reason, r.consecutive_failures),
            ("ok", None, 0)
        );
    }

    /// `v_contribution_health` counts the contribution's pending dead letters
    /// (by its consumer, or events about its extension) and marks an overdue
    /// schedule unfresh.
    #[tokio::test]
    async fn the_view_counts_dead_letters_and_marks_a_missed_schedule() {
        let db = Database::in_memory();
        let k = key();
        db.transaction({
            let k = k.clone();
            move |tx| {
                set_next_due_tx(tx, &k, Some("2000-01-01T00:00:00.000000Z"), "t1")?;
                for (seq, subject) in [(1, "[\"extension:tracker\"]"), (2, "[]"), (3, "[]")] {
                    tx.execute(
                        "INSERT INTO event_log (seq, id, type, v, at, source, subject, payload)
                         VALUES (?1, 'e' || ?1, 'x.y', 1, 't', 's', ?2, '{}')",
                        rusqlite::params![seq, subject],
                    )
                    .map_err(map_sql_err)?;
                }
                for (consumer, seq, state) in [
                    ("some.consumer", 1, "pending"),
                    ("extension:tracker/fake", 2, "pending"),
                    ("extension:tracker/fake", 3, "discarded"),
                ] {
                    tx.execute(
                        "INSERT INTO event_dead_letter (consumer, event_seq, error, first_failed_at, last_failed_at, state)
                         VALUES (?1, ?2, 'boom', 't', 't', ?3)",
                        rusqlite::params![consumer, seq, state],
                    )
                    .map_err(map_sql_err)?;
                }
                Ok(())
            }
        })
        .await
        .unwrap();
        let (letters, fresh): (i64, i64) = db
            .read(|c| {
                c.query_row(
                    "SELECT dead_letters, fresh FROM v_contribution_health",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!((letters, fresh), (2, 0));
    }
}
