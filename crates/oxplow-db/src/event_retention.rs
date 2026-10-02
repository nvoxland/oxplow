//! Retention for the event log's bodies (P3.11,
//! `.context/target-architecture.md` §5.4). Envelopes are tiny and are
//! what timelines and attribution rely on, so they are kept; payloads and
//! large content (tool input and output) are the bulk and the privacy
//! exposure, so they expire per namespace:
//!
//! | Namespace | Payload | Large content |
//! |---|---|---|
//! | `agent` | 30 days | 14 days |
//! | `test`, `code`, `collector`, `effect` | 90 days | 30 days |
//! | a plugin's (any namespace core doesn't own) | 30 days | 14 days |
//! | core's state (`snapshot`, `vcs`, `effort`, `work_item`, `command`, `config`, …) | kept | kept |
//!
//! An expired payload is replaced by `{}` and stamped `payload_expired_at`
//! (the column is NOT NULL); an expired body's row is deleted, and a
//! `read_event_content` of its hash reads as gone. Per-project settings
//! for these windows are a later phase; these are the spec's defaults.

use oxplow_domain::{DomainError, Timestamp};
use rusqlite::{params, OptionalExtension};

use crate::database::{map_sql_err, ts_to_string, Database};

pub const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// Core's windows: `(namespace, payload days, large-content days)`. A
/// core namespace not listed is state, kept whole.
pub const POLICY: &[(&str, i64, i64)] = &[
    ("agent", 30, 14),
    ("test", 90, 30),
    ("code", 90, 30),
    ("collector", 90, 30),
    ("effect", 90, 30),
];

/// Every plugin namespace's window (§5.4): payloads 30 days, large
/// content 14. A plugin's own, shorter window comes with its declared
/// event types (P8).
pub const PLUGIN_DEFAULT: (i64, i64) = (30, 14);

/// The namespaces the sweep expires and their windows: core's
/// [`POLICY`], then every namespace in the log or the content store
/// that core doesn't own, at [`PLUGIN_DEFAULT`].
async fn windows(db: &Database) -> Result<Vec<(String, i64, i64)>, DomainError> {
    let mut out: Vec<(String, i64, i64)> = POLICY
        .iter()
        .map(|(ns, p, c)| (ns.to_string(), *p, *c))
        .collect();
    for ns in plugin_namespaces(db).await? {
        out.push((ns, PLUGIN_DEFAULT.0, PLUGIN_DEFAULT.1));
    }
    Ok(out)
}

/// The namespaces with live payloads or stored bodies that core doesn't
/// own. The log's are found by skipping through the live-payload index a
/// namespace at a time (one probe each), never by scanning it.
async fn plugin_namespaces(db: &Database) -> Result<Vec<String>, DomainError> {
    db.read(|tx| {
        let mut found = std::collections::BTreeSet::new();
        let mut after = String::new();
        loop {
            let next: Option<String> = tx
                .query_row(
                    "SELECT type FROM event_log INDEXED BY event_log_live_payload
                      WHERE payload_expired_at IS NULL AND type > ?1
                      ORDER BY type LIMIT 1",
                    [&after],
                    |r| r.get(0),
                )
                .optional()
                .map_err(map_sql_err)?;
            let Some(t) = next else { break };
            let ns = t.split('.').next().unwrap_or_default().to_string();
            // Past every `<ns>.` type: `/` is the byte after `.`.
            after = format!("{ns}/");
            found.insert(ns);
        }
        let mut st = tx
            .prepare("SELECT DISTINCT namespace FROM event_content")
            .map_err(map_sql_err)?;
        for ns in st
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(map_sql_err)?
        {
            found.insert(ns.map_err(map_sql_err)?);
        }
        Ok(found
            .into_iter()
            .filter(|ns| !oxplow_domain::events::schema::CORE_NAMESPACES.contains(&ns.as_str()))
            .collect())
    })
    .await
}

/// What one sweep removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SweepReport {
    pub content_deleted: usize,
    pub payloads_expired: usize,
}

/// Rows one sweep transaction touches: small enough that a hook waiting on
/// the writer lock waits milliseconds, not the whole backlog.
pub const BATCH: i64 = 5000;

/// Apply [`POLICY`] and the plugin default as of `now`, in transactions
/// of at most [`BATCH`]
/// rows, so the first sweep over a large old log never holds the writer
/// lock for long. Idempotent: an already-expired payload or an
/// already-deleted body is not counted again.
pub async fn sweep(db: &Database, now: Timestamp) -> Result<SweepReport, DomainError> {
    sweep_in_batches(db, now, BATCH).await
}

async fn sweep_in_batches(
    db: &Database,
    now: Timestamp,
    batch: i64,
) -> Result<SweepReport, DomainError> {
    let mut report = SweepReport::default();
    let stamp = ts_to_string(now);
    let before = |days: i64| ts_to_string(Timestamp::from_unix_ms(now.unix_ms() - days * DAY_MS));
    for (ns, payload_days, content_days) in windows(db).await? {
        let (ns, cutoff) = (ns.to_string(), before(content_days));
        loop {
            let (ns, cutoff) = (ns.clone(), cutoff.clone());
            let n = db
                .transaction(move |tx| {
                    tx.execute(
                        "DELETE FROM event_content WHERE hash IN (
                           SELECT hash FROM event_content
                            WHERE namespace = ?1 AND created_at < ?2 LIMIT ?3)",
                        params![ns, cutoff, batch],
                    )
                    .map_err(map_sql_err)
                })
                .await?;
            report.content_deleted += n;
            if (n as i64) < batch {
                break;
            }
        }
        // `<ns>.` up to `<ns>/` (the next byte after `.`) is exactly the
        // namespace's types, as an index range.
        let (lo, hi, cutoff) = (format!("{ns}."), format!("{ns}/"), before(payload_days));
        loop {
            let (lo, hi, cutoff, stamp) = (lo.clone(), hi.clone(), cutoff.clone(), stamp.clone());
            let n = db
                .transaction(move |tx| {
                    tx.execute(
                        "UPDATE event_log SET payload = '{}', payload_expired_at = ?4
                          WHERE seq IN (
                            SELECT seq FROM event_log INDEXED BY event_log_live_payload
                             WHERE payload_expired_at IS NULL
                               AND type >= ?1 AND type < ?2 AND at < ?3
                             LIMIT ?5)",
                        params![lo, hi, cutoff, stamp, batch],
                    )
                    .map_err(map_sql_err)
                })
                .await?;
            report.payloads_expired += n;
            if (n as i64) < batch {
                break;
            }
        }
    }
    // A parked event whose payload expired can never be retried.
    db.transaction(|tx| {
        tx.execute(
            "UPDATE event_dead_letter SET state = 'discarded'
              WHERE state = 'pending'
                AND EXISTS (SELECT 1 FROM event_log e
                             WHERE e.seq = event_seq AND e.payload_expired_at IS NOT NULL)",
            [],
        )
        .map_err(map_sql_err)
    })
    .await?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Database;

    /// A backlog larger than one batch is swept in several transactions,
    /// all of it, and nothing outside the namespace's type range moves.
    #[tokio::test]
    async fn a_backlog_is_swept_in_batches() {
        let db = Database::in_memory();
        let now = oxplow_domain::Timestamp::from_unix_ms(100 * DAY_MS);
        let old = crate::database::ts_to_string(oxplow_domain::Timestamp::from_unix_ms(
            now.unix_ms() - 31 * DAY_MS,
        ));
        db.transaction(move |tx| {
            for i in 0..5 {
                tx.execute(
                    "INSERT INTO event_log (id, type, v, at, source, subject, payload)
                       VALUES (?1, 'agent.tool.finished', 1, ?2, 'test', '[]', '{\"tool\":\"Bash\"}')",
                    rusqlite::params![format!("e{i}"), old],
                )
                .map_err(crate::map_sql_err)?;
                tx.execute(
                    "INSERT INTO event_content (hash, namespace, bytes, size, created_at)
                       VALUES (?1, 'agent', x'00', 1, ?2)",
                    rusqlite::params![format!("h{i}"), old],
                )
                .map_err(crate::map_sql_err)?;
            }
            // `agentx.` sorts inside `agent%` but is another namespace — a
            // plugin's, whose 29-day-old payload is within its 30 days.
            let young = crate::database::ts_to_string(oxplow_domain::Timestamp::from_unix_ms(
                now.unix_ms() - 29 * DAY_MS,
            ));
            tx.execute(
                "INSERT INTO event_log (id, type, v, at, source, subject, payload)
                   VALUES ('x', 'agentx.thing', 1, ?1, 'test', '[]', '{\"a\":1}')",
                [&young],
            )
            .map_err(crate::map_sql_err)?;
            Ok(())
        })
        .await
        .unwrap();
        let report = sweep_in_batches(&db, now, 2).await.unwrap();
        assert_eq!(
            report,
            SweepReport {
                content_deleted: 5,
                payloads_expired: 5
            }
        );
        let kept: String = db
            .transaction(|tx| {
                tx.query_row("SELECT payload FROM event_log WHERE id = 'x'", [], |r| {
                    r.get(0)
                })
                .map_err(crate::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(kept, "{\"a\":1}");
    }

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

    /// P7.B7: a plugin's namespace expires on the plugin default (payload
    /// 30 days, body 14); `collector` and `effect` payloads keep 90 days;
    /// core's state namespaces are still kept whole.
    #[tokio::test]
    async fn plugin_namespaces_expire_on_the_default_and_collectors_keep_90_days() {
        let db = Database::in_memory();
        let now = oxplow_domain::Timestamp::from_unix_ms(400 * DAY_MS);
        let days_ago = |d: i64| {
            crate::database::ts_to_string(oxplow_domain::Timestamp::from_unix_ms(
                now.unix_ms() - d * DAY_MS,
            ))
        };
        let rows = [
            ("p-old", "acme.thing.done", days_ago(31)),
            ("p-new", "acme.thing.done", days_ago(29)),
            ("c-old", "collector.synced", days_ago(91)),
            ("c-new", "collector.synced", days_ago(89)),
            ("s-old", "snapshot.taken", days_ago(300)),
        ];
        let bodies = [
            ("b-old", "acme", days_ago(15)),
            ("b-new", "acme", days_ago(13)),
        ];
        db.transaction(move |tx| {
            for (id, ty, at) in &rows {
                tx.execute(
                    "INSERT INTO event_log (id, type, v, at, source, subject, payload)
                       VALUES (?1, ?2, 1, ?3, 'test', '[]', '{\"x\":1}')",
                    rusqlite::params![id, ty, at],
                )
                .map_err(crate::map_sql_err)?;
            }
            for (hash, ns, at) in &bodies {
                tx.execute(
                    "INSERT INTO event_content (hash, namespace, bytes, size, created_at)
                       VALUES (?1, ?2, x'00', 1, ?3)",
                    rusqlite::params![hash, ns, at],
                )
                .map_err(crate::map_sql_err)?;
            }
            Ok(())
        })
        .await
        .unwrap();
        sweep(&db, now).await.unwrap();
        let expired: Vec<String> = db
            .read(|tx| {
                let mut st = tx
                    .prepare(
                        "SELECT id FROM event_log WHERE payload_expired_at IS NOT NULL ORDER BY id",
                    )
                    .map_err(crate::map_sql_err)?;
                let r = st
                    .query_map([], |r| r.get(0))
                    .map_err(crate::map_sql_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(crate::map_sql_err)?;
                Ok(r)
            })
            .await
            .unwrap();
        assert_eq!(expired, vec!["c-old", "p-old"]);
        let bodies_left: Vec<String> = db
            .read(|tx| {
                let mut st = tx
                    .prepare("SELECT hash FROM event_content ORDER BY hash")
                    .map_err(crate::map_sql_err)?;
                let r = st
                    .query_map([], |r| r.get(0))
                    .map_err(crate::map_sql_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(crate::map_sql_err)?;
                Ok(r)
            })
            .await
            .unwrap();
        assert_eq!(bodies_left, vec!["b-new"]);
    }
}
