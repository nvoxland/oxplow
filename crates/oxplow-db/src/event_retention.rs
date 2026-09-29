//! Retention for the event log's bodies (P3.11,
//! `.context/target-architecture.md` §5.4). Envelopes are tiny and are
//! what timelines and attribution rely on, so they are kept; payloads and
//! large content (tool input and output) are the bulk and the privacy
//! exposure, so they expire per namespace:
//!
//! | Namespace | Payload | Large content |
//! |---|---|---|
//! | `agent` | 30 days | 14 days |
//! | `test`, `code` | 90 days | 30 days |
//! | everything else (state: `snapshot`, `vcs`, `effort`, `work_item`, `command`, `config`, `effect`, …) | kept | kept |
//!
//! An expired payload is replaced by `{}` and stamped `payload_expired_at`
//! (the column is NOT NULL); an expired body's row is deleted, and a
//! `read_event_content` of its hash reads as gone. Per-project settings
//! for these windows are a later phase; these are the spec's defaults.

use oxplow_domain::{DomainError, Timestamp};
use rusqlite::params;

use crate::database::{map_sql_err, ts_to_string, Database};

pub const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// `(namespace, payload days, large-content days)`.
pub const POLICY: &[(&str, i64, i64)] = &[("agent", 30, 14), ("test", 90, 30), ("code", 90, 30)];

/// What one sweep removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepReport {
    pub content_deleted: usize,
    pub payloads_expired: usize,
}

/// Apply [`POLICY`] as of `now`. Idempotent: an already-expired payload
/// or an already-deleted body is not counted again.
pub async fn sweep(db: &Database, now: Timestamp) -> Result<SweepReport, DomainError> {
    db.transaction(move |tx| {
        let mut report = SweepReport::default();
        let stamp = ts_to_string(now);
        for (ns, payload_days, content_days) in POLICY {
            let before =
                |days: i64| ts_to_string(Timestamp::from_unix_ms(now.unix_ms() - days * DAY_MS));
            report.content_deleted += tx
                .execute(
                    "DELETE FROM event_content WHERE namespace = ?1 AND created_at < ?2",
                    params![ns, before(*content_days)],
                )
                .map_err(map_sql_err)?;
            report.payloads_expired += tx
                .execute(
                    "UPDATE event_log SET payload = '{}', payload_expired_at = ?3
                      WHERE type LIKE ?1 AND at < ?2 AND payload_expired_at IS NULL",
                    params![format!("{ns}.%"), before(*payload_days), stamp],
                )
                .map_err(map_sql_err)?;
        }
        // A parked event whose payload expired can never be retried.
        tx.execute(
            "UPDATE event_dead_letter SET state = 'discarded'
              WHERE state = 'pending'
                AND EXISTS (SELECT 1 FROM event_log e
                             WHERE e.seq = event_seq AND e.payload_expired_at IS NOT NULL)",
            [],
        )
        .map_err(map_sql_err)?;
        Ok(report)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    /// A parked event whose payload expires can never be retried, so its
    /// pending dead letter is discarded with it.
    #[tokio::test]
    async fn a_pending_dead_letter_of_an_expired_event_is_discarded() {
        let db = Database::in_memory();
        let now = oxplow_domain::Timestamp::from_unix_ms(100 * DAY_MS);
        let old = crate::database::ts_to_string(oxplow_domain::Timestamp::from_unix_ms(
            now.unix_ms() - 31 * DAY_MS,
        ));
        db.transaction(move |tx| {
            for (id, ty) in [("e1", "agent.tool.finished"), ("e2", "effort.closed")] {
                tx.execute(
                    "INSERT INTO event_log (id, type, v, at, source, subject, payload)
                       VALUES (?1, ?2, 1, ?3, 'test', '[]', '{\"tool\":\"Bash\"}')",
                    rusqlite::params![id, ty, old],
                )
                .map_err(crate::map_sql_err)?;
            }
            for seq in [1, 2] {
                crate::event_log_store::dead_letter_tx(tx, "rec", seq, "boom")?;
            }
            Ok(())
        })
        .await
        .unwrap();
        sweep(&db, now).await.unwrap();
        let states: Vec<(i64, String)> = db
            .transaction(|tx| {
                let mut s = tx
                    .prepare("SELECT event_seq, state FROM event_dead_letter ORDER BY event_seq")
                    .map_err(crate::map_sql_err)?;
                let r = s
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map_err(crate::map_sql_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(crate::map_sql_err)?;
                Ok(r)
            })
            .await
            .unwrap();
        assert_eq!(
            states,
            vec![(1, "discarded".to_string()), (2, "pending".to_string())]
        );
    }

    /// P3.11 (tsk481): an agent tool body older than 14 days is gone and its
    /// event older than 30 days keeps its envelope with the payload
    /// replaced; state events are kept whole; a second sweep does nothing.
    #[tokio::test]
    async fn old_agent_bodies_expire_and_state_events_stay_whole() {
        let db = Database::in_memory();
        let now = oxplow_domain::Timestamp::from_unix_ms(100 * DAY_MS);
        let days_ago = |d: i64| {
            crate::database::ts_to_string(oxplow_domain::Timestamp::from_unix_ms(
                now.unix_ms() - d * DAY_MS,
            ))
        };
        let (old15, old31, old2) = (days_ago(15), days_ago(31), days_ago(2));
        db.transaction(move |tx| {
            for (hash, ns, at) in [("h-old", "agent", &old15), ("h-new", "agent", &old2), ("h-test", "test", &old15)] {
                tx.execute(
                    "INSERT INTO event_content (hash, namespace, bytes, size, created_at) VALUES (?1, ?2, x'00', 1, ?3)",
                    rusqlite::params![hash, ns, at],
                )
                .map_err(crate::map_sql_err)?;
            }
            for (id, ty, at) in [
                ("e1", "agent.tool.finished", &old31),
                ("e2", "agent.tool.finished", &old2),
                ("e3", "effort.closed", &old31),
            ] {
                tx.execute(
                    "INSERT INTO event_log (id, type, v, at, source, subject, payload)
                       VALUES (?1, ?2, 1, ?3, 'test', '[]', '{\"tool\":\"Bash\"}')",
                    rusqlite::params![id, ty, at],
                )
                .map_err(crate::map_sql_err)?;
            }
            Ok(())
        })
        .await
        .unwrap();

        let first = sweep(&db, now).await.unwrap();
        assert_eq!((first.content_deleted, first.payloads_expired), (1, 1));
        let again = sweep(&db, now).await.unwrap();
        assert_eq!(
            (again.content_deleted, again.payloads_expired),
            (0, 0),
            "idempotent"
        );

        let rows: Vec<(String, String, Option<String>)> = db
            .transaction(|tx| {
                let mut s = tx
                    .prepare("SELECT id, payload, payload_expired_at FROM event_log ORDER BY id")
                    .map_err(crate::map_sql_err)?;
                let r = s
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .map_err(crate::map_sql_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(crate::map_sql_err)?;
                Ok(r)
            })
            .await
            .unwrap();
        assert_eq!(rows[0].1, "{}", "a 31-day-old agent payload is replaced");
        assert!(rows[0].2.is_some());
        assert_eq!(rows[1].1, "{\"tool\":\"Bash\"}", "a recent one stays");
        assert_eq!(
            rows[2].1, "{\"tool\":\"Bash\"}",
            "state events are kept whole"
        );
        let left: Vec<String> = db
            .transaction(|tx| {
                let mut s = tx
                    .prepare("SELECT hash FROM event_content ORDER BY hash")
                    .map_err(crate::map_sql_err)?;
                let r = s
                    .query_map([], |r| r.get(0))
                    .map_err(crate::map_sql_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(crate::map_sql_err)?;
                Ok(r)
            })
            .await
            .unwrap();
        assert_eq!(left, vec!["h-new", "h-test"], "test bodies keep 30 days");
    }
}
