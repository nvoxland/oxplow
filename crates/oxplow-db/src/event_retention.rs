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
//! | a plugin's (any namespace core doesn't own) | 30 days, or its declared window | 14 days, or its declared window |
//! | core's state (`snapshot`, `vcs`, `effort`, `work_item`, `command`, `config`, …) | kept | kept |
//!
//! The windows are `oxplow_domain::events::retention`'s. A project may set
//! its own per namespace (`eventRetention`, a person's key): it replaces
//! core's default, and for a plugin's namespace it is capped at the
//! plugin's window (tsk947).
//!
//! An expired payload is replaced by `{}` and stamped `payload_expired_at`
//! (the column is NOT NULL); an expired body's row is deleted, and a
//! `read_event_content` of its hash reads as gone.

use std::collections::BTreeMap;

use oxplow_domain::events::retention::{RetentionWindow, CORE_WINDOWS, PLUGIN_DEFAULT};
use oxplow_domain::{DomainError, Timestamp};
use rusqlite::{params, OptionalExtension};

use crate::database::{map_sql_err, ts_to_string, Database};

pub const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// Whether an extension may declare this window: at least a day, and no
/// longer than [`PLUGIN_DEFAULT`] — a plugin may keep its rows for less,
/// never more.
pub fn check_declared(payload_days: i64, content_days: i64) -> Result<(), String> {
    let (p, c) = (PLUGIN_DEFAULT.payload_days, PLUGIN_DEFAULT.content_days);
    if payload_days < 1 || content_days < 1 {
        return Err("retention windows are at least 1 day".into());
    }
    if payload_days > p {
        return Err(format!(
            "`payload_days: {payload_days}` is longer than the {p}-day default; a plugin may \
             only keep its events for less"
        ));
    }
    if content_days > c {
        return Err(format!(
            "`content_days: {content_days}` is longer than the {c}-day default; a plugin may \
             only keep its events for less"
        ));
    }
    Ok(())
}

/// An extension present in the catalog, with its declared window (`None`:
/// the default).
#[derive(Debug, Clone, PartialEq)]
pub struct DeclaredRetention {
    pub namespace: String,
    pub extension: String,
    /// `(payload days, content days)`.
    pub window: Option<(i64, i64)>,
}

/// Restate the declared windows from the extensions present: each one's
/// window recorded, or dropped when it declares none. An extension not
/// present keeps the window it last declared.
pub fn restate_declared_tx(
    conn: &rusqlite::Connection,
    present: &[DeclaredRetention],
    now: &str,
) -> Result<(), DomainError> {
    for d in present {
        match d.window {
            Some((payload_days, content_days)) => conn.execute(
                "INSERT INTO plugin_event_retention
                     (namespace, extension, payload_days, content_days, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (namespace) DO UPDATE SET
                     extension = excluded.extension,
                     payload_days = excluded.payload_days,
                     content_days = excluded.content_days,
                     updated_at = excluded.updated_at",
                params![d.namespace, d.extension, payload_days, content_days, now],
            ),
            None => conn.execute(
                "DELETE FROM plugin_event_retention WHERE namespace = ?1",
                [&d.namespace],
            ),
        }
        .map_err(map_sql_err)?;
    }
    Ok(())
}

/// The namespaces the sweep expires and their windows: core's
/// [`CORE_WINDOWS`], each replaced by the project's when it sets one; then
/// every namespace in the log or the content store that core doesn't own,
/// at its declared window or [`PLUGIN_DEFAULT`] — the project's when
/// shorter, never longer.
async fn windows(
    db: &Database,
    project: &BTreeMap<String, RetentionWindow>,
) -> Result<Vec<(String, RetentionWindow)>, DomainError> {
    let mut out: Vec<(String, RetentionWindow)> = CORE_WINDOWS
        .iter()
        .map(|(ns, default)| (ns.to_string(), *project.get(*ns).unwrap_or(default)))
        .collect();
    let declared: std::collections::HashMap<String, (i64, i64)> = db
        .read(|tx| {
            let mut st = tx
                .prepare("SELECT namespace, payload_days, content_days FROM plugin_event_retention")
                .map_err(map_sql_err)?;
            let rows = st
                .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))
                .map_err(map_sql_err)?
                .collect::<rusqlite::Result<_>>()
                .map_err(map_sql_err)?;
            Ok(rows)
        })
        .await?;
    for ns in plugin_namespaces(db).await? {
        let plugin = declared
            .get(&ns)
            .map_or(PLUGIN_DEFAULT, |(p, c)| RetentionWindow::new(*p, *c));
        let window = project.get(&ns).map_or(plugin, |w| w.at_most(plugin));
        out.push((ns, window));
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

/// Apply the windows ([`windows`], with the project's `project`) as of
/// `now`, in transactions of at most [`BATCH`] rows, so the first sweep
/// over a large old log never holds the writer lock for long. Idempotent:
/// an already-expired payload or an already-deleted body is not counted
/// again.
pub async fn sweep(
    db: &Database,
    now: Timestamp,
    project: &BTreeMap<String, RetentionWindow>,
) -> Result<SweepReport, DomainError> {
    sweep_in_batches(db, now, project, BATCH).await
}

async fn sweep_in_batches(
    db: &Database,
    now: Timestamp,
    project: &BTreeMap<String, RetentionWindow>,
    batch: i64,
) -> Result<SweepReport, DomainError> {
    let mut report = SweepReport::default();
    let stamp = ts_to_string(now);
    let before = |days: i64| ts_to_string(Timestamp::from_unix_ms(now.unix_ms() - days * DAY_MS));
    for (ns, window) in windows(db, project).await? {
        let (payload_days, content_days) = (window.payload_days, window.content_days);
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
        let report = sweep_in_batches(&db, now, &BTreeMap::new(), 2)
            .await
            .unwrap();
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

    /// P8.D5: an extension's declared window replaces the plugin default
    /// for its namespace, and is kept when the extension goes; one present
    /// without a window goes back to the default.
    #[tokio::test]
    async fn a_declared_window_expires_its_namespace_sooner_and_outlives_the_extension() {
        let db = Database::in_memory();
        let now = oxplow_domain::Timestamp::from_unix_ms(100 * DAY_MS);
        let days_ago = |d: i64| {
            crate::database::ts_to_string(oxplow_domain::Timestamp::from_unix_ms(
                now.unix_ms() - d * DAY_MS,
            ))
        };
        let (old8, old6) = (days_ago(8), days_ago(6));
        db.transaction(move |tx| {
            restate_declared_tx(
                tx,
                &[DeclaredRetention {
                    namespace: "acme_pr".into(),
                    extension: "acme-pr".into(),
                    window: Some((7, 3)),
                }],
                "t",
            )?;
            // The extension is gone: its window stays.
            restate_declared_tx(tx, &[], "t")?;
            for (id, at) in [("old", &old8), ("new", &old6)] {
                tx.execute(
                    "INSERT INTO event_log (id, type, v, at, source, subject, payload)
                       VALUES (?1, 'acme_pr.merged', 1, ?2, 'test', '[]', '{\"number\":1}')",
                    rusqlite::params![id, at],
                )
                .map_err(crate::map_sql_err)?;
            }
            Ok(())
        })
        .await
        .unwrap();
        let report = sweep(&db, now, &BTreeMap::new()).await.unwrap();
        assert_eq!(report.payloads_expired, 1, "only the 8-day-old payload");

        // Back without a window: the default (30 days) again.
        db.transaction(|tx| {
            restate_declared_tx(
                tx,
                &[DeclaredRetention {
                    namespace: "acme_pr".into(),
                    extension: "acme-pr".into(),
                    window: None,
                }],
                "t",
            )?;
            let n: i64 = tx
                .query_row("SELECT count(*) FROM plugin_event_retention", [], |r| {
                    r.get(0)
                })
                .map_err(crate::map_sql_err)?;
            assert_eq!(n, 0);
            Ok(())
        })
        .await
        .unwrap();
    }

    /// Seed `agent.tool.finished` and `acme_pr.merged` payloads `days`
    /// old, by id.
    async fn seed_payloads(db: &Database, now: Timestamp, rows: &[(&str, &str, i64)]) {
        let rows: Vec<(String, String, String)> = rows
            .iter()
            .map(|(id, ty, days)| {
                let at = crate::database::ts_to_string(Timestamp::from_unix_ms(
                    now.unix_ms() - days * DAY_MS,
                ));
                (id.to_string(), ty.to_string(), at)
            })
            .collect();
        db.transaction(move |tx| {
            for (id, ty, at) in &rows {
                tx.execute(
                    "INSERT INTO event_log (id, type, v, at, source, subject, payload)
                       VALUES (?1, ?2, 1, ?3, 'test', '[]', '{\"a\":1}')",
                    rusqlite::params![id, ty, at],
                )
                .map_err(crate::map_sql_err)?;
            }
            Ok(())
        })
        .await
        .unwrap();
    }

    async fn expired(db: &Database) -> Vec<String> {
        db.read(|tx| {
            let mut st = tx
                .prepare(
                    "SELECT id FROM event_log WHERE payload_expired_at IS NOT NULL ORDER BY id",
                )
                .map_err(crate::map_sql_err)?;
            let ids = st
                .query_map([], |r| r.get(0))
                .map_err(crate::map_sql_err)?
                .collect::<rusqlite::Result<Vec<String>>>()
                .map_err(crate::map_sql_err)?;
            Ok(ids)
        })
        .await
        .unwrap()
    }

    /// tsk947: a project's window for a core namespace replaces core's
    /// default — longer or shorter.
    #[tokio::test]
    async fn a_project_window_overrides_the_default() {
        let db = Database::in_memory();
        let now = Timestamp::from_unix_ms(200 * DAY_MS);
        seed_payloads(
            &db,
            now,
            &[
                ("a40", "agent.tool.finished", 40),
                ("a70", "agent.tool.finished", 70),
                ("t20", "test.run.recorded", 20),
            ],
        )
        .await;
        let project = BTreeMap::from([
            ("agent".to_string(), RetentionWindow::new(60, 14)),
            ("test".to_string(), RetentionWindow::new(10, 5)),
        ]);
        sweep(&db, now, &project).await.unwrap();
        // agent: kept 60 days, not 30; test: 10, not 90.
        assert_eq!(expired(&db).await, vec!["a70", "t20"]);
    }

    /// tsk947: a project can keep a plugin's events for less, never for
    /// longer than its extension declared (or the plugin default).
    #[tokio::test]
    async fn a_plugin_namespace_cant_be_kept_longer_by_a_project() {
        let db = Database::in_memory();
        let now = Timestamp::from_unix_ms(200 * DAY_MS);
        db.transaction(|tx| {
            restate_declared_tx(
                tx,
                &[DeclaredRetention {
                    namespace: "acme_pr".into(),
                    extension: "acme-pr".into(),
                    window: Some((7, 3)),
                }],
                "t",
            )
        })
        .await
        .unwrap();
        seed_payloads(
            &db,
            now,
            &[
                ("p5", "acme_pr.merged", 5),
                ("p8", "acme_pr.merged", 8),
                ("o20", "other_ns.thing", 20),
                ("o35", "other_ns.thing", 35),
            ],
        )
        .await;
        let longer = BTreeMap::from([
            ("acme_pr".to_string(), RetentionWindow::new(90, 90)),
            ("other_ns".to_string(), RetentionWindow::new(90, 90)),
        ]);
        sweep(&db, now, &longer).await.unwrap();
        assert_eq!(expired(&db).await, vec!["o35", "p8"], "capped at 7 and 30");
        let shorter = BTreeMap::from([("acme_pr".to_string(), RetentionWindow::new(3, 1))]);
        sweep(&db, now, &shorter).await.unwrap();
        assert_eq!(expired(&db).await, vec!["o35", "p5", "p8"]);
    }

    #[test]
    fn a_declared_window_may_only_be_shorter() {
        assert!(check_declared(7, 3).is_ok());
        assert!(check_declared(30, 14).is_ok());
        let long = check_declared(60, 14).unwrap_err();
        assert!(long.contains("60") && long.contains("30"), "{long}");
        assert!(check_declared(7, 15).is_err());
        assert!(check_declared(0, 1).is_err());
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
        sweep(&db, now, &BTreeMap::new()).await.unwrap();
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

        let first = sweep(&db, now, &BTreeMap::new()).await.unwrap();
        assert_eq!((first.content_deleted, first.payloads_expired), (1, 1));
        let again = sweep(&db, now, &BTreeMap::new()).await.unwrap();
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
        sweep(&db, now, &BTreeMap::new()).await.unwrap();
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
