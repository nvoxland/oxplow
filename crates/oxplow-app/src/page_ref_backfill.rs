//! The page-ref graph restated from its SQLite sources at boot.
//!
//! The writers in `oxplow-db` mirror their outbound refs into
//! `page_ref` on every save, so between boots the graph follows its
//! sources. A migration that resets `page_ref` (V92 rewrote its kinds
//! and cleared it) leaves every unsaved row without its edges, and a
//! writer's bug leaves drift. This restates every task, link, effort,
//! finding and note slice from its row on boot, so the graph is its
//! sources' again — which is why it stays (tsk920): it is the graph's
//! one repair path.
//!
//! It runs only when the graph may have drifted from its sources: after a
//! migration (one may reset it) or under a new build (a writer's bug may
//! have been fixed). Between those, the writers keep it current, so
//! restating every row on each boot repeated what was already there. Its
//! last run is recorded as the `page_ref_repair` row of `asset_state`
//! (`built_from`: the build and schema version), read as `v_asset`.
//!
//! Ordering doesn't matter — projections are per-source and each
//! writer owns its own slice. The backfill is idempotent: running
//! it again replaces the same rows it wrote last time.
//!
//! Wiki bodies and recent commits are NOT re-projected here —
//! the wiki watcher's initial scan and the commit indexer's boot
//! pass already hit those paths. This module only covers the
//! kinds whose data lives entirely in SQLite (tasks, links,
//! efforts, findings).

use std::sync::Arc;

use oxplow_db::page_ref_projections::{
    finding_edges, link_edge, note_edges, task_body_ref_types, task_edges, task_link_ref_types,
    work_item_id, KIND_FINDING, KIND_TASK_NOTE, KIND_WORK_ITEM,
};
use oxplow_db::{
    SqliteCodeQualityStore, SqliteEffortStore, SqlitePageRefStore, SqliteTaskLinkStore,
    SqliteTaskNoteStore, SqliteTaskStore,
};
use oxplow_domain::stores::TaskLinkStore as _;
use oxplow_domain::vocabulary::VocabularyHandle;

/// Counts of rows touched per kind. Logged at INFO so the boot
/// trail makes the backfill observable.
#[derive(Debug, Default)]
pub struct BackfillCounts {
    pub tasks: usize,
    pub links: usize,
    pub efforts: usize,
    pub findings: usize,
    pub notes: usize,
}

/// The `asset_state` row recording the last repair.
pub const REPAIR: &str = "page_ref_repair";

/// What a repair is current for: this build of the program and the
/// database's schema version.
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
            "build": oxplow_db::table_generations::build_identity(),
            "schema": schema,
        })
        .to_string(),
    )
}

/// Whether the graph needs restating: no repair yet by this build at this
/// schema version (or that can't be told).
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

/// Project every existing row into `page_ref`. Idempotent.
pub async fn run(
    vocabulary: VocabularyHandle,
    page_refs: Arc<SqlitePageRefStore>,
    tasks: Arc<SqliteTaskStore>,
    links: Arc<SqliteTaskLinkStore>,
    efforts: Arc<SqliteEffortStore>,
    findings_store: Arc<SqliteCodeQualityStore>,
    task_note: Arc<SqliteTaskNoteStore>,
) -> BackfillCounts {
    let mut counts = BackfillCounts::default();
    let vocabulary = vocabulary.current();
    let kinds = &vocabulary.kinds;

    // 1. task body slice + touched-file slice.
    if let Ok(items) = tasks.list_all_for_backfill().await {
        for item in items {
            let edges = task_edges(kinds, &item);
            let id_str = work_item_id(item.id);
            if let Err(e) = page_refs
                .replace_source_for_ref_types(KIND_WORK_ITEM, &id_str, task_body_ref_types(), edges)
                .await
            {
                tracing::warn!(?e, id = %item.id, "page-ref backfill: task failed");
                continue;
            }
            counts.tasks += 1;
        }
    }

    // 1b. The effort-owned slice (touched files, summary mentions,
    //     declared impacts) of every work item with an effort — an oxplow
    //     task's or another provider's — through the store's own projector.
    if let Ok(work_items) = efforts.list_work_items().await {
        for work_item in work_items {
            match efforts.project_effort_slice(&work_item).await {
                Ok(()) => counts.efforts += 1,
                Err(e) => tracing::warn!(?e, %work_item, "page-ref backfill: effort slice failed"),
            }
        }
    }

    // 2. Link slice — re-project the union of outgoing links per
    //    distinct from-item. (Each link contributes one edge; we
    //    write the whole slice owned by the source in one shot so
    //    deletions on the live path stay clean too.)
    if let Ok(from_items) = links.list_distinct_from_items().await {
        for from in from_items {
            let outgoing = match links.list_outgoing(from).await {
                Ok(v) => v,
                Err(_) => continue,
            };
            let edges: Vec<_> = outgoing.iter().map(link_edge).collect();
            let from_str = work_item_id(from);
            if let Err(e) = page_refs
                .replace_source_for_ref_types(
                    KIND_WORK_ITEM,
                    &from_str,
                    task_link_ref_types(),
                    edges,
                )
                .await
            {
                tracing::warn!(?e, id = %from, "page-ref backfill: link slice failed");
                continue;
            }
            counts.links += 1;
        }
    }

    // 3. Work notes — one source per note row, parsed from body.
    if let Ok(rows) = task_note.list_all_for_backfill().await {
        for (id, body) in rows {
            let edges = note_edges(kinds, &id, &body);
            if page_refs
                .replace_source(KIND_TASK_NOTE, &id, edges)
                .await
                .is_ok()
            {
                counts.notes += 1;
            }
        }
    }

    // 4. Findings — one edge per row.
    if let Ok(rows) = findings_store.list_all_findings_for_backfill().await {
        for (id, path) in rows {
            let id_str = id.to_string();
            let edges = finding_edges(&id_str, &path);
            if page_refs
                .replace_source(KIND_FINDING, &id_str, edges)
                .await
                .is_ok()
            {
                counts.findings += 1;
            }
        }
    }

    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_db::Database;
    use oxplow_domain::stores::{StreamStore, TaskStore, ThreadStore};
    use oxplow_domain::{
        Stream, StreamId, StreamKind, Task, TaskActorKind, TaskAuthor, TaskId, TaskPriority,
        TaskStatus, Thread, ThreadId, ThreadStatus, Timestamp,
    };

    fn ts() -> Timestamp {
        Timestamp::from_unix_ms(1_700_000_000_000)
    }

    /// The repair is needed once per build and schema version: recorded,
    /// it isn't again until either changes.
    #[tokio::test]
    async fn the_repair_runs_once_per_build_and_schema() {
        let db = Database::in_memory();
        assert!(needs_repair(&db).await, "never repaired");
        record_repair(&db, 5).await;
        assert!(!needs_repair(&db).await, "repaired by this build");
        db.transaction(|tx| {
            tx.execute(
                "UPDATE asset_state SET built_from = '{\"build\":\"older\",\"schema\":1}' WHERE asset = ?1",
                [REPAIR],
            )
            .map_err(oxplow_db::map_sql_err)
        })
        .await
        .unwrap();
        assert!(needs_repair(&db).await, "another build or schema");
    }

    #[tokio::test]
    async fn backfill_picks_up_pre_existing_task_refs() {
        let db = Database::in_memory();

        // Every store mirrors into the ref graph at write time now —
        // a "bare" writer can't exist. Simulate pre-migration data by
        // inserting normally, then clearing the projected slice below.
        let streams = oxplow_db::SqliteStreamStore::new(db.clone());
        let threads = oxplow_db::SqliteThreadStore::new(db.clone());
        let bare_items = SqliteTaskStore::new(db.clone());

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
                status: ThreadStatus::Active,
                sort_index: 0,
                pane_target: "working".into(),
                agent: oxplow_domain::AgentKind::Claude,
                acp_agent: None,
                resume_session_id: String::new(),
                summary: String::new(),
                summary_updated_at: None,
                closed_at: None,
                custom_prompt: None,
                created_at: ts(),
                updated_at: ts(),
                archived_at: None,
            })
            .await
            .unwrap();
        let task_id = bare_items
            .insert(&Task {
                id: TaskId::placeholder(),
                thread_id: Some(ThreadId::new(1)),
                parent_id: None,
                title: "fix".into(),
                description: "see [[src/app.rs]]".into(),
                status: TaskStatus::Ready,
                priority: TaskPriority::Medium,
                sort_index: 0,
                created_by: TaskActorKind::User,
                created_at: ts(),
                updated_at: ts(),
                completed_at: None,
                deleted_at: None,
                note_count: 0,
                author: Some(TaskAuthor::User),
            })
            .await
            .unwrap();

        // An effort on the task that declared an impact, and one on another
        // provider's work item with a summary mention (tsk452).
        use oxplow_db::EffortStore as _;
        let effort_writer = SqliteEffortStore::new(db.clone());
        let own = effort_writer
            .start(
                &oxplow_domain::refs::build::work_item_ref(task_id),
                &ThreadId::new(1),
                None,
            )
            .await
            .unwrap();
        effort_writer
            .set_impacts(
                &own.id,
                &[oxplow_domain::TaskImpact {
                    kind: "wiki".into(),
                    id: "auth-flow".into(),
                    action: Some("updated".into()),
                }],
            )
            .await
            .unwrap();
        let foreign = effort_writer
            .start("work_item:issues:ENG-12", &ThreadId::new(1), None)
            .await
            .unwrap();
        effort_writer
            .finish(&foreign.id, None, Some("touched [[src/lib.rs]]".into()))
            .await
            .unwrap();

        let page_refs = Arc::new(SqlitePageRefStore::new(db.clone()));
        // Wipe the slices the writes just projected so the table looks
        // like a DB written before ref mirroring existed.
        for source in [format!("oxplow:{task_id}"), "issues:ENG-12".to_string()] {
            page_refs
                .replace_source("work_item", &source, Vec::new())
                .await
                .unwrap();
        }
        let pre = page_refs
            .list_backlinks("file", "src/app.rs", None)
            .await
            .unwrap();
        assert!(pre.is_empty());

        // Build the attached stores the backfill consumes.
        let items_attached = Arc::new(SqliteTaskStore::new(db.clone()));
        let links = Arc::new(SqliteTaskLinkStore::new(db.clone()));
        let efforts = Arc::new(SqliteEffortStore::new(db.clone()));
        let findings_store = Arc::new(SqliteCodeQualityStore::new(db.clone()));
        let notes = Arc::new(SqliteTaskNoteStore::new(db.clone()));

        let counts = run(
            VocabularyHandle::core(),
            page_refs.clone(),
            items_attached,
            links,
            efforts,
            findings_store,
            notes,
        )
        .await;
        assert!(counts.tasks >= 1);

        let post = page_refs
            .list_backlinks("file", "src/app.rs", None)
            .await
            .unwrap();
        assert_eq!(post.len(), 1, "got {post:?}");
        assert_eq!(post[0].source_id, format!("oxplow:{task_id}"));
        // Another provider's work item is backfilled too …
        let foreign_refs = page_refs
            .list_backlinks("file", "src/lib.rs", None)
            .await
            .unwrap();
        assert_eq!(
            foreign_refs
                .iter()
                .map(|r| r.source_id.as_str())
                .collect::<Vec<_>>(),
            vec!["issues:ENG-12"]
        );
        // … and a declared impact survives the backfill.
        let impacts = page_refs
            .list_backlinks("wiki", "auth-flow", None)
            .await
            .unwrap();
        assert_eq!(impacts.len(), 1, "got {impacts:?}");
    }
}
