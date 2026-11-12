//! The page-ref graph restated from its SQLite sources at boot.
//!
//! The writers in `oxplow-db` mirror their outbound refs into
//! `page_ref` on every save, so between boots the graph follows its
//! sources. A migration that resets `page_ref` (V92 rewrote its kinds
//! and cleared it) leaves every unsaved row without its edges, and a
//! writer's bug leaves drift. This restates every task (from the
//! task interface, the same core its pump consumer runs),
//! finding, thread-note and handoff slice from its row on boot, so the graph is its
//! sources' again — which is why it stays (tsk920): it is the graph's
//! one repair path.
//!
//! It runs only when the graph may have drifted from its sources: after a
//! migration (one may reset it) or when the code that projects edges
//! changed (a writer's bug may have been fixed). Between those, the
//! writers keep it current, so restating every row on each boot repeated
//! what was already there. Its last run is recorded as the
//! `page_ref_repair` row of `asset_state` (`built_from`: the projection
//! code's hash and the schema version), read as `v_asset`. It used to be
//! keyed on the build, so every dev rebuild repaid it (~59 s of a thread
//! at boot), though projections rarely change.
//!
//! It writes in batches ([`BATCH`] sources per transaction): every commit
//! is a change event the UI hears, and a commit per row — a quarter of a
//! million findings once — flooded it for a minute at boot. And it writes
//! only what drifted: a source whose stored edges equal its projection is
//! left alone (`replace_sources`), so a run that finds the graph current
//! (the projection key hashes source text, so a comment edit runs it) is
//! reads, not a rewrite of every finding's edge.
//!
//! Ordering doesn't matter — projections are per-source and each
//! writer owns its own slice. The backfill is idempotent: running
//! it again replaces the same rows it wrote last time.
//!
//! Knowledge page bodies and recent commits are NOT re-projected here —
//! the oxwiki watcher's initial scan and the commit indexer's boot
//! pass already hit those paths. This module only covers the
//! kinds whose data lives entirely in SQLite (tasks, findings,
//! handoffs).

use std::sync::Arc;

use oxplow_db::page_ref_projections::{body_edges, finding_edges};
use oxplow_db::{SourceSlice, SqliteCodeQualityStore, SqliteHandoffStore, SqlitePageRefStore};
use oxplow_domain::vocabulary::VocabularyHandle;

/// Counts of rows touched per kind. Logged at INFO so the boot
/// trail makes the backfill observable.
#[derive(Debug, Default)]
pub struct BackfillCounts {
    pub findings: usize,
    pub handoffs: usize,
}

/// Sources restated per transaction.
const BATCH: usize = 1000;

/// Write `slices` in batches; how many were written. A failed batch is
/// logged and skipped (its rows keep whatever edges they had).
async fn write_batched(
    page_refs: &SqlitePageRefStore,
    what: &str,
    slices: Vec<SourceSlice>,
) -> usize {
    let mut written = 0;
    let mut slices = slices.into_iter().peekable();
    while slices.peek().is_some() {
        let batch: Vec<SourceSlice> = slices.by_ref().take(BATCH).collect();
        let n = batch.len();
        match page_refs.replace_sources(batch).await {
            Ok(()) => written += n,
            Err(e) => tracing::warn!(?e, what, "page-ref backfill: a batch failed"),
        }
    }
    written
}

fn slice(
    source: String,
    ref_types: Option<Vec<String>>,
    edges: Vec<oxplow_db::PageRefEdge>,
) -> SourceSlice {
    SourceSlice {
        source,
        ref_types,
        edges,
    }
}

/// The `asset_state` row recording the last repair.
pub const REPAIR: &str = "page_ref_repair";

/// The code that decides a source's edges: this module, the shared
/// projections and the task interface's projector. When any of it changes, the graph may have drifted; an
/// edit elsewhere can't move it.
const PROJECTION_SOURCES: &[&str] = &[
    include_str!("page_ref_backfill.rs"),
    include_str!("../../oxplow-db/src/page_ref_projections.rs"),
];

/// A hash of [`PROJECTION_SOURCES`]: the same for every build of the same
/// projection code.
fn projections_identity() -> String {
    let mut hasher = xxhash_rust::xxh3::Xxh3::new();
    for source in PROJECTION_SOURCES {
        hasher.update(source.as_bytes());
    }
    format!("{:016x}", hasher.digest())
}

/// What a repair is current for: the projection code and the database's
/// schema version.
async fn repair_key(db: &oxplow_db::Database) -> Option<String> {
    let schema: i64 = db
        .read(|tx| {
            tx.query_row(
                "SELECT coalesce(max(version), 0) FROM refinery_schema_history",
                [],
                |r| r.get(0),
            )
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .ok()?;
    Some(
        serde_json::json!({
            "projections": projections_identity(),
            "schema": schema,
        })
        .to_string(),
    )
}

/// Whether the graph needs restating: no repair yet with this projection
/// code at this schema version (or that can't be told).
pub async fn needs_repair(db: &oxplow_db::Database) -> bool {
    let Some(key) = repair_key(db).await else {
        return true;
    };
    let stored: Option<String> = db
        .read(|tx| {
            use rusqlite::OptionalExtension;
            tx.query_row(
                "SELECT built_from FROM asset_state WHERE asset = ?1",
                [REPAIR],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .map(Option::flatten)
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .ok()
        .flatten();
    stored.as_deref() != Some(key.as_str())
}

/// Record a repair that took `elapsed_ms`.
pub async fn record_repair(db: &oxplow_db::Database, elapsed_ms: i64) {
    let Some(key) = repair_key(db).await else {
        return;
    };
    let at = oxplow_domain::Timestamp::now().to_string();
    let recorded = db
        .transaction(move |tx| {
            tx.execute(
                "INSERT INTO asset_state (asset, computed_at, events_to, elapsed_ms, built_from)
                 VALUES (?1, ?2, (SELECT coalesce(max(seq), 0) FROM event_log), ?3, ?4)
                 ON CONFLICT (asset) DO UPDATE SET
                    computed_at = excluded.computed_at, events_to = excluded.events_to,
                    elapsed_ms = excluded.elapsed_ms, built_from = excluded.built_from",
                rusqlite::params![REPAIR, at, elapsed_ms, key],
            )
            .map(|_| ())
            .map_err(oxplow_db::map_sql_err)
        })
        .await;
    if let Err(error) = recorded {
        tracing::warn!(%error, "recording the page-ref repair failed");
    }
}

/// What the backfill projects: the stores of the kinds whose data lives
/// entirely in SQLite.
pub struct Sources {
    pub findings: Arc<SqliteCodeQualityStore>,
    pub handoffs: Arc<SqliteHandoffStore>,
}

/// Project every existing row into `page_ref`. Idempotent.
pub async fn run(
    vocabulary: VocabularyHandle,
    page_refs: Arc<SqlitePageRefStore>,
    sources: Sources,
) -> BackfillCounts {
    let Sources {
        findings: findings_store,
        handoffs,
    } = sources;
    let mut counts = BackfillCounts::default();
    let vocabulary = vocabulary.current();
    let kinds = &vocabulary.resources;

    // 2b. Handoffs — one source per row, parsed from its body, as a note.
    if let Ok(rows) = handoffs.list_all_for_backfill().await {
        let slices = rows
            .into_iter()
            .map(|(uri, body)| {
                let edges = body_edges(kinds, &uri, &body);
                slice(uri, None, edges)
            })
            .collect();
        counts.handoffs = write_batched(&page_refs, "handoffs", slices).await;
    }

    // 3. Findings — one edge per row.
    if let Ok(rows) = findings_store.list_all_findings_for_backfill().await {
        let slices = rows
            .into_iter()
            .map(|(id, path)| {
                let id = id.to_string();
                let edges = finding_edges(&id, &path);
                slice(oxplow_domain::uri::build::finding_uri(&id), None, edges)
            })
            .collect();
        counts.findings = write_batched(&page_refs, "findings", slices).await;
    }

    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::Database;
    use oxplow_domain::stores::{StreamStore, ThreadStore};
    use oxplow_domain::{Stream, StreamId, StreamKind, Thread, ThreadId, Timestamp};
    use oxplow_oxtask::OxtaskId;
    use oxplow_oxtask::OxtaskStore;
    use oxplow_oxtask::{Oxtask, OxtaskActorKind, OxtaskAuthor, OxtaskPriority, OxtaskStatus};

    fn ts() -> Timestamp {
        Timestamp::from_unix_ms(1_700_000_000_000)
    }

    /// The repair is needed once per projection code and schema version:
    /// recorded, it isn't again until either changes. A rebuild that left
    /// the projections alone doesn't repay it (it took ~59 s of a thread
    /// on every dev rebuild when it was keyed on the build).
    #[tokio::test]
    async fn the_repair_runs_once_per_projection_code_and_schema() {
        let db = Database::in_memory();
        assert!(needs_repair(&db).await, "never repaired");
        record_repair(&db, 5).await;
        assert!(
            !needs_repair(&db).await,
            "repaired with this projection code"
        );
        let stored: String = db
            .read(|tx| {
                tx.query_row(
                    "SELECT built_from FROM asset_state WHERE asset = ?1",
                    [REPAIR],
                    |r| r.get(0),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        let key: serde_json::Value = serde_json::from_str(&stored).unwrap();
        assert_eq!(key["projections"], projections_identity(), "{key}");
        assert!(key.get("build").is_none(), "not the build: {key}");
        db.transaction(|tx| {
            tx.execute(
                "UPDATE asset_state SET built_from = json_set(built_from, '$.projections', 'older') WHERE asset = ?1",
                [REPAIR],
            )
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap();
        assert!(needs_repair(&db).await, "other projection code");
    }

    /// The repair restates rows in batches: thousands of findings are a
    /// handful of commits, not one each — each commit is a change event the
    /// UI hears, and one per row flooded it at boot.
    #[tokio::test]
    async fn the_repair_writes_in_batches() {
        let db = Database::in_memory();
        let findings_store = Arc::new(SqliteCodeQualityStore::new(db.clone()));
        let scan = findings_store
            .create_scan("duplication", "change 1", "working", "x")
            .await
            .unwrap();
        let rows = (0..2500)
            .map(|i| oxplow_db::CodeQualityFinding {
                id: 0,
                scan_id: scan,
                path: format!("src/f{i}.rs"),
                start_line: 1,
                end_line: 9,
                kind: "duplicate-block".into(),
                metric_value: 9.0,
                extra_json: None,
            })
            .collect();
        findings_store
            .finish_scan_with_findings(scan, rows)
            .await
            .unwrap();
        let page_refs = Arc::new(SqlitePageRefStore::new(db.clone()));
        // Lose the edges, as a migration resetting the graph would.
        db.transaction(|tx| {
            tx.execute("DELETE FROM page_ref", [])
                .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap();

        let mut changes = db.subscribe_changes();
        let counts = run(
            VocabularyHandle::core(),
            page_refs.clone(),
            Sources {
                db: db.clone(),
                findings: findings_store,
                handoffs: Arc::new(SqliteHandoffStore::new(db.clone())),
            },
        )
        .await;
        assert_eq!(counts.findings, 2500);
        assert_eq!(
            page_refs
                .list_backlinks(&oxplow_domain::uri::build::file_uri("src/f42.rs"), None)
                .await
                .unwrap()
                .len(),
            1
        );
        let mut published = 0;
        loop {
            match changes.try_recv() {
                Ok(_) => published += 1,
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(n)) => {
                    published += n as usize
                }
                Err(_) => break,
            }
        }
        assert!(
            published <= 10,
            "{published} change messages for one repair"
        );
    }

    /// A repair over a graph that is already its sources' writes no edge:
    /// every source whose stored edges equal its projection is left alone,
    /// so a repair that finds no drift (the usual case: a projection file's
    /// comment changed) is reads, not a quarter of a million rewrites the
    /// UI hears.
    #[tokio::test]
    async fn a_repair_over_a_current_graph_writes_no_edge() {
        let db = Database::in_memory();
        let findings_store = Arc::new(SqliteCodeQualityStore::new(db.clone()));
        let scan = findings_store
            .create_scan("duplication", "change 1", "working", "x")
            .await
            .unwrap();
        let rows = (0..1500)
            .map(|i| oxplow_db::CodeQualityFinding {
                id: 0,
                scan_id: scan,
                path: format!("src/f{i}.rs"),
                start_line: 1,
                end_line: 9,
                kind: "duplicate-block".into(),
                metric_value: 9.0,
                extra_json: None,
            })
            .collect();
        findings_store
            .finish_scan_with_findings(scan, rows)
            .await
            .unwrap();
        let page_refs = Arc::new(SqlitePageRefStore::new(db.clone()));
        let sources = || Sources {
            db: db.clone(),
            findings: findings_store.clone(),
            handoffs: Arc::new(SqliteHandoffStore::new(db.clone())),
        };
        // One finding lost its edge: the repair restores it, and only it.
        db.transaction(|tx| {
            tx.execute(
                "DELETE FROM page_ref WHERE target_uri = ?1",
                [oxplow_domain::uri::build::file_uri("src/f7.rs")],
            )
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap();
        run(VocabularyHandle::core(), page_refs.clone(), sources()).await;
        assert_eq!(
            page_refs
                .list_backlinks(&oxplow_domain::uri::build::file_uri("src/f7.rs"), None)
                .await
                .unwrap()
                .len(),
            1,
            "the drifted edge is restored"
        );

        let mut changes = db.subscribe_changes();
        let counts = run(VocabularyHandle::core(), page_refs.clone(), sources()).await;
        assert_eq!(counts.findings, 1500, "every finding is still checked");
        loop {
            match changes.try_recv() {
                Ok(changed) => assert!(
                    !changed.contains("page_ref"),
                    "a current graph was rewritten: {changed:?}"
                ),
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(n)) => {
                    panic!("{n} change messages lagged for a repair that changed nothing")
                }
                Err(_) => break,
            }
        }
    }

    /// Every task is restated from the interface, whichever list
    /// it's on: its body, its links (the list's own types) and its
    /// comments.
    #[tokio::test]
    async fn backfill_restates_every_task_from_the_interface() {
        let db = Database::in_memory();

        // Every store mirrors into the ref graph at write time now —
        // a "bare" writer can't exist. Simulate pre-migration data by
        // inserting normally, then clearing the projected slice below.
        let streams = oxplow_db::SqliteStreamStore::new(db.clone());
        let threads = oxplow_db::SqliteThreadStore::new(db.clone());
        let bare_items = oxplow_oxtask::SqliteOxtaskStore::new(db.clone());

        streams
            .upsert(&Stream {
                id: StreamId::new(1),
                kind: StreamKind::Primary,
                title: "x".into(),
                branch: "main".into(),
                branch_ref: "refs/heads/main".into(),
                branch_source: "main".into(),
                worktree_path: "/r".into(),
                working_pane: String::new(),
                talking_pane: String::new(),
                working_session_id: String::new(),
                talking_session_id: String::new(),
                host: oxplow_domain::HostId::LOCAL,
                custom_prompt: None,
                created_at: ts(),
                updated_at: ts(),
                archived_at: None,
            })
            .await
            .unwrap();
        threads
            .upsert(&Thread {
                id: ThreadId::new(1),
                stream_id: StreamId::new(1),
                title: "x".into(),
                sort_index: 0,
                summary: String::new(),
                summary_updated_at: None,
                closed_at: None,
                custom_prompt: None,
                created_at: ts(),
                updated_at: ts(),
                archived_at: None,
                mark: None,
            })
            .await
            .unwrap();
        let task_id = bare_items
            .insert(&Oxtask {
                id: OxtaskId::placeholder(),
                parent_id: None,
                title: "fix".into(),
                description: "see [[src/app.rs]]".into(),
                status: OxtaskStatus::Ready,
                priority: OxtaskPriority::Medium,
                created_by: OxtaskActorKind::User,
                created_at: ts(),
                updated_at: ts(),
                completed_at: None,
                deleted_at: None,
                note_count: 0,
                author: Some(OxtaskAuthor::User),
            })
            .await
            .unwrap();
        // Its row in the interface, as its record put it there: its stage's
        // bucket is the list's declaration.
        let stages = serde_json::to_string(&crate::capabilities::built_in_stage_decls(
            crate::capabilities::built_in("oxplow:oxtask").unwrap(),
        ))
        .unwrap();
        db.transaction(move |tx| {
            use crate::event_pump::EventConsumer as _;
            tx.execute(
                "INSERT INTO capability_provider
                   (capability, provider, features_json, active, title, source, available,
                    capability_title, choosable, optional, stages_json)
                 VALUES ('task_tracker', 'oxtask', '{}', 1, 'oxtask', 'builtin', 1,
                         'Task tracker', 1, 1, ?1)",
                [&stages],
            )
            .map_err(oxplow_db::map_sql_err)?;
            let record = oxplow_oxtask::record::record_tx(tx, task_id)?;
            crate::tasks::TasksProjection.handle(
                tx,
                &oxplow_domain::StoredEvent {
                    seq: 1,
                    envelope: oxplow_oxtask::provider::recorded(record),
                    payload_expired_at: None,
                },
            )
        })
        .await
        .unwrap();

        // Another list's item, as its records left it in the interface.
        db.transaction(|tx| {
            for sql in [
                "INSERT INTO task (uri, provider, title, body, state, stage, created_at, updated_at)
                 VALUES ('oxplow:oxplow/task?id=ENG-12&provider=issues', 'issues', 'Theirs', '', 'todo', 'Todo', 't', 't')",
                "INSERT INTO task_link (from_uri, to_uri, link_type, created_at)
                 VALUES ('oxplow:oxplow/task?id=ENG-12&provider=issues', 'oxplow:oxplow/task?id=ENG-7&provider=issues', 'parent_of', 't')",
                "INSERT INTO task_comment (id, uri, body, created_at)
                 VALUES ('oxplow:oxplow/task?id=ENG-12&provider=issues#part=c1', 'oxplow:oxplow/task?id=ENG-12&provider=issues', 'see [[src/c.rs]]', 't')",
            ] {
                tx.execute(sql, []).map_err(oxplow_db::map_sql_err)?;
            }
            Ok(())
        })
        .await
        .unwrap();
        let page_refs = Arc::new(SqlitePageRefStore::new(db.clone()));
        // Wipe the slices the writes just projected so the table looks
        // like a DB whose graph was reset.
        for source in [
            oxplow_domain::uri::build::task_uri("oxtask", &task_id.to_string()),
            oxplow_domain::uri::build::task_uri("issues", "ENG-12"),
        ] {
            page_refs.replace_source(&source, Vec::new()).await.unwrap();
        }
        let pre = page_refs
            .list_backlinks(&oxplow_domain::uri::build::file_uri("src/app.rs"), None)
            .await
            .unwrap();
        assert!(pre.is_empty());

        let counts = run(
            VocabularyHandle::core(),
            page_refs.clone(),
            Sources {
                db: db.clone(),
                findings: Arc::new(SqliteCodeQualityStore::new(db.clone())),
                handoffs: Arc::new(SqliteHandoffStore::new(db.clone())),
            },
        )
        .await;
        assert_eq!(counts.tasks, 2);

        let post = page_refs
            .list_backlinks(&oxplow_domain::uri::build::file_uri("src/app.rs"), None)
            .await
            .unwrap();
        assert_eq!(post.len(), 1, "got {post:?}");
        assert_eq!(
            post[0].source,
            oxplow_domain::uri::build::task_uri("oxtask", &task_id.to_string())
        );
        // Another provider's task is backfilled too — its link and its
        // comment.
        use oxplow_domain::uri::build;
        for (target, ref_type) in [
            (build::task_uri("issues", "ENG-7"), "task_link:parent_of"),
            (build::file_uri("src/c.rs"), "comment_file_ref"),
        ] {
            let refs = page_refs.list_backlinks(&target, None).await.unwrap();
            assert_eq!(
                refs.iter()
                    .map(|r| (r.source.clone(), r.ref_type.as_str()))
                    .collect::<Vec<_>>(),
                vec![(build::task_uri("issues", "ENG-12"), ref_type)],
                "{target}"
            );
        }
    }
}
