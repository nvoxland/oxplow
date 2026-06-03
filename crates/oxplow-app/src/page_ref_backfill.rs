//! One-shot boot-time backfill for the unified page-ref graph.
//!
//! The writers in `oxplow-db` mirror their outbound refs into
//! `page_ref` on every save, but pre-existing data (rows that
//! existed before the migration ran) doesn't get touched until
//! someone re-saves it. This module re-projects every relevant
//! row exactly once on boot so backlinks for an upgraded DB
//! aren't blank until the user starts editing.
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
    effort_ref_types, effort_summary_edges, effort_touched_file_edges, finding_edges, link_edge,
    note_edges, task_body_ref_types, task_edges, task_link_ref_types, KIND_FINDING, KIND_TASK,
    KIND_TASK_NOTE,
};
use oxplow_db::TaskEffortStore as _;
use oxplow_db::{
    SqliteCodeQualityStore, SqlitePageRefStore, SqliteTaskEffortStore, SqliteTaskLinkStore,
    SqliteTaskNoteStore, SqliteTaskStore,
};
use oxplow_domain::stores::TaskLinkStore as _;

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

/// Project every existing row into `page_ref`. Idempotent.
pub async fn run(
    page_refs: Arc<SqlitePageRefStore>,
    tasks: Arc<SqliteTaskStore>,
    links: Arc<SqliteTaskLinkStore>,
    efforts: Arc<SqliteTaskEffortStore>,
    findings_store: Arc<SqliteCodeQualityStore>,
    task_note: Arc<SqliteTaskNoteStore>,
) -> BackfillCounts {
    let mut counts = BackfillCounts::default();

    // 1. task body slice + touched-file slice.
    if let Ok(items) = tasks.list_all_for_backfill().await {
        for item in items {
            let edges = task_edges(&item);
            let id_str = item.id.to_string();
            if let Err(e) = page_refs
                .replace_source_for_ref_types(KIND_TASK, &id_str, task_body_ref_types(), edges)
                .await
            {
                tracing::warn!(?e, id = %item.id, "page-ref backfill: task failed");
                continue;
            }
            counts.tasks += 1;
            // Effort-owned slice = touched-file union ∪ parsed
            // refs from every summary body. For touched files, the
            // most-recent effort's `change_kind` wins per path so
            // the renderer can display "created"/"modified"/
            // "deleted" rather than a flat "touched".
            use std::collections::BTreeMap;
            let mut paths: BTreeMap<String, String> = BTreeMap::new();
            let mut summaries: Vec<String> = Vec::new();
            if let Ok(item_efforts) = efforts.list_for_item(item.id).await {
                // list_for_item is sorted started_at DESC; walking
                // in reverse gives oldest-first so the latest write
                // wins via plain `insert`.
                let efforts_oldest_first: Vec<_> = item_efforts.into_iter().rev().collect();
                for ef in efforts_oldest_first {
                    if let Some(s) = &ef.summary {
                        if !s.trim().is_empty() {
                            summaries.push(s.clone());
                        }
                    }
                    if let Ok(rows) = efforts.list_files(&ef.id).await {
                        for r in rows {
                            let kind_str = match r.change {
                                oxplow_db::EffortFileChange::Created => "created",
                                oxplow_db::EffortFileChange::Updated => "updated",
                                oxplow_db::EffortFileChange::Deleted => "deleted",
                            };
                            paths.insert(r.path, kind_str.to_string());
                        }
                    }
                }
            }
            let path_vec: Vec<(String, String)> = paths.into_iter().collect();
            let mut edges = effort_touched_file_edges(&id_str, &path_vec);
            edges.extend(effort_summary_edges(&id_str, &summaries));
            let had_payload = !path_vec.is_empty() || !summaries.is_empty();
            let _ = page_refs
                .replace_source_for_ref_types(KIND_TASK, &id_str, effort_ref_types(), edges)
                .await;
            if had_payload {
                counts.efforts += 1;
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
            let from_str = from.to_string();
            if let Err(e) = page_refs
                .replace_source_for_ref_types(KIND_TASK, &from_str, task_link_ref_types(), edges)
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
            let edges = note_edges(&id, &body);
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

    #[tokio::test]
    async fn backfill_picks_up_pre_existing_task_refs() {
        let db = Database::in_memory();

        // Construct stores WITHOUT page_refs first so writes don't
        // mirror — this simulates pre-migration data.
        let streams = oxplow_db::SqliteStreamStore::new(db.clone());
        let threads = oxplow_db::SqliteThreadStore::new(db.clone());
        let bare_items = SqliteTaskStore::new(db.clone());

        streams
            .upsert(&Stream {
                id: StreamId::from("s-1"),
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
                id: ThreadId::from("b-1"),
                stream_id: StreamId::from("s-1"),
                title: "x".into(),
                status: ThreadStatus::Active,
                sort_index: 0,
                pane_target: "working".into(),
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
                thread_id: Some(ThreadId::from("b-1")),
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

        let page_refs = Arc::new(SqlitePageRefStore::new(db.clone()));
        // No backlinks for the file yet — writer was bare.
        let pre = page_refs
            .list_backlinks("file", "src/app.rs", None)
            .await
            .unwrap();
        assert!(pre.is_empty());

        // Build the attached stores the backfill consumes.
        let items_attached =
            Arc::new(SqliteTaskStore::new(db.clone()).with_page_refs((*page_refs).clone()));
        let links =
            Arc::new(SqliteTaskLinkStore::new(db.clone()).with_page_refs((*page_refs).clone()));
        let efforts =
            Arc::new(SqliteTaskEffortStore::new(db.clone()).with_page_refs((*page_refs).clone()));
        let findings_store =
            Arc::new(SqliteCodeQualityStore::new(db.clone()).with_page_refs((*page_refs).clone()));
        let notes =
            Arc::new(SqliteTaskNoteStore::new(db.clone()).with_page_refs((*page_refs).clone()));

        let counts = run(
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
        assert_eq!(post[0].source_id, task_id.to_string());
    }
}
