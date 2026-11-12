//! Says which pinned sources a snapshot drifted (`.context/models.md`
//! "Markdown bodies"): one `snapshot.ref_drifted` per source whose
//! `file_pin` names a file the take changed, or whose `dir_pin` names a
//! directory a changed file is under, caused by the `snapshot.taken`.
//! Staleness is the primary stream's (`v_ref_pin`), so another stream's
//! take is nothing; so is a take that changed no pinned file. What counts
//! stale items follows these instead of every snapshot.

use oxplow_db::EventCtx;
use oxplow_domain::events::schema::{RefDrifted, RefDriftedV1, SnapshotTaken, SnapshotTakenV3};
use oxplow_domain::DomainError;

fn sql(e: rusqlite::Error) -> DomainError {
    oxplow_db::map_sql_err(e)
}

pub struct RefDrift {
    pub vocabulary: oxplow_domain::vocabulary::VocabularyHandle,
}

impl RefDrift {
    pub const NAME: &'static str = "refs.drift";
}

impl crate::event_pump::EventConsumer for RefDrift {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == <SnapshotTaken as oxplow_domain::EventType>::TYPE
    }

    fn handle(
        &self,
        conn: &rusqlite::Connection,
        event: &oxplow_domain::StoredEvent,
    ) -> Result<(), DomainError> {
        let Ok(taken) = serde_json::from_value::<SnapshotTakenV3>(event.envelope.payload.clone())
        else {
            return Ok(());
        };
        if taken.unchanged || taken.file_count == 0 {
            return Ok(());
        }
        let mut stmt = conn
            .prepare_cached(
                "SELECT pr.source_uri, fs.path
                   FROM snapshot s
                   JOIN streams st ON st.id = s.stream_id AND st.kind = 'primary'
                   JOIN file_snapshot fs ON fs.snapshot_id = s.id
                   JOIN page_ref pr
                     ON (pr.ref_type = 'file_pin' AND uri_param(pr.target_uri, 'path') = fs.path)
                     OR (pr.ref_type = 'dir_pin' AND fs.path LIKE uri_param(pr.target_uri, 'path') || '/%')
                  WHERE s.id = ?1
                  ORDER BY pr.source_uri, fs.path",
            )
            .map_err(sql)?;
        let mut drifted: std::collections::BTreeMap<String, Vec<String>> = Default::default();
        for row in stmt
            .query_map([taken.snapshot], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(sql)?
        {
            let (source, path) = row.map_err(sql)?;
            let paths = drifted.entry(source).or_default();
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
        if drifted.is_empty() {
            return Ok(());
        }
        let vocabulary = self.vocabulary.current();
        let ev = EventCtx {
            vocabulary: &vocabulary,
            source: event.envelope.source.clone(),
            cause: Some(event.envelope.id.clone()),
        };
        for (source, paths) in drifted {
            let env = ev
                .typed::<RefDrifted>(&RefDriftedV1 {
                    source: source.clone(),
                    snapshot: taken.snapshot,
                    paths,
                })
                .with_anchors(event.envelope.anchors.clone())
                .with_subject([source]);
            ev.append(conn, &env)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use oxplow_domain::events::schema::{RefDrifted, RefDriftedV1, SnapshotTaken, SnapshotTakenV3};

    /// A primary-stream take that changed a file a source pins, or a file
    /// under a directory it pins, drifts that source; one that changed no
    /// pinned file says nothing.
    #[tokio::test]
    async fn a_take_that_changes_a_pinned_file_drifts_its_source() {
        let fx = crate::test_fixtures::services_with_thread().await;
        let (file_src, dir_src) = (
            "oxplow:oxplow/knowledge?id=a&store=s",
            "oxplow:oxplow/knowledge?id=b&store=s",
        );
        fx.svc
            .db
            .transaction(move |tx| {
                for (src, target, ty) in [
                    (file_src, "oxplow:oxplow/file?path=src/a.rs", "file_pin"),
                    (dir_src, "oxplow:oxplow/dir?path=src", "dir_pin"),
                ] {
                    oxplow_db::page_ref_store::upsert_edge_tx(
                        tx,
                        &oxplow_db::PageRefEdge::new(src, target, ty),
                    )?;
                }
                Ok(())
            })
            .await
            .unwrap();
        let snapshot_with = |path: &'static str| {
            let db = fx.svc.db.clone();
            async move {
                db.transaction(move |c| {
                    c.execute(
                        "INSERT INTO snapshot (stream_id, created_at)
                         SELECT id, '2026-09-30T00:00:00.000000Z' FROM streams WHERE kind = 'primary'",
                        [],
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                    let id = c.last_insert_rowid();
                    c.execute(
                        "INSERT INTO file_snapshot (stream_id, path, blob_hash, size_bytes, captured_at, storage, snapshot_id)
                         SELECT id, ?1, 'h', 1, '2026-09-30T00:00:00.000000Z', 'oxplow', ?2
                         FROM streams WHERE kind = 'primary'",
                        rusqlite::params![path, id],
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                    Ok(id)
                })
                .await
                .unwrap()
            }
        };
        let vocabulary = fx.svc.vocabulary.current();
        let taken = |snapshot: i64| {
            oxplow_db::EventCtx {
                vocabulary: &vocabulary,
                source: "snapshot".into(),
                cause: None,
            }
            .typed::<SnapshotTaken>(&SnapshotTakenV3 {
                stream: "str1".into(),
                snapshot,
                parent: None,
                trigger: oxplow_domain::snapshot::SnapshotTrigger::Quiet,
                unchanged: false,
                file_count: 1,
                elapsed_ms: 1,
                budget_ms: None,
                over_budget: false,
            })
        };
        let pinned = snapshot_with("src/a.rs").await;
        let other = snapshot_with("docs/x.md").await;
        fx.svc.event_log_store.append(taken(pinned)).await.unwrap();
        fx.svc.event_log_store.append(taken(other)).await.unwrap();
        fx.svc.event_pump.run_once().await.unwrap();
        let drifted: Vec<RefDriftedV1> = fx
            .svc
            .event_log_store
            .read_after(0, 1000)
            .await
            .unwrap()
            .into_iter()
            .filter(|e| e.envelope.event_type == <RefDrifted as oxplow_domain::EventType>::TYPE)
            .map(|e| serde_json::from_value(e.envelope.payload).unwrap())
            .collect();
        assert_eq!(
            drifted,
            vec![
                RefDriftedV1 {
                    source: file_src.into(),
                    snapshot: pinned,
                    paths: vec!["src/a.rs".into()]
                },
                RefDriftedV1 {
                    source: dir_src.into(),
                    snapshot: pinned,
                    paths: vec!["src/a.rs".into()]
                },
            ]
        );
    }
}
