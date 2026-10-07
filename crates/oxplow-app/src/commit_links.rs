//! A commit is linked to the task it was for by what it holds (tsk1035).
//!
//! An agent's commit rarely names its task — and the agent rule is to name
//! tasks by title, never id — so the message can't be the link. The work
//! can: a commit **holds** an effort's work when they share a file and the
//! commit's version of every file they share is the effort's end version
//! (its end snapshot's bytes, as the object they'd be). A partial commit of
//! an effort's files holds it too.
//!
//! The link is a `work_item → commit` `page_ref` edge (`RT_COMMITTED`, the
//! effort in `source_extra`) — the same shape as a declared impact — so
//! `v_commit_task` and the commit's backlinks read it. Made from both sides:
//! when a commit is indexed, against the efforts that closed before it
//! ([`link_commit`]); and when an effort finishes, against the commits made
//! since it started ([`link_effort`]: committed before the task was closed).

use std::path::Path;

use oxplow_db::page_ref_projections::{KIND_COMMIT, KIND_WORK_ITEM, RT_COMMITTED};
use oxplow_db::{Effort, EffortStore as _, PageRefEdge};
use oxplow_domain::stores::ThreadStore as _;
use oxplow_domain::vcs::ObjectId;
use oxplow_domain::{DomainError, EffortId, Timestamp};

use crate::snapshot_files::SnapshotFileError;

/// How far back a commit looks for the efforts whose work it may hold.
const LOOKBACK_MS: i64 = 14 * 24 * 60 * 60 * 1000;

/// Whether a commit holds an effort's work, given each file they share as
/// `(the commit's object, the object the effort's end version would be)` —
/// `None` where the file is absent on that side.
pub fn holds(shared: &[(Option<ObjectId>, Option<ObjectId>)]) -> bool {
    !shared.is_empty() && shared.iter().all(|(commit, effort)| commit == effort)
}

/// Link commit `sha` (just indexed from workspace `ws`, committed at
/// `committed`) to each effort that closed before it, in the last two weeks,
/// whose work it holds.
pub async fn link_commit(
    svc: &crate::Services,
    ws: &Path,
    sha: &str,
    committed: Timestamp,
) -> Result<(), DomainError> {
    // A commit is stamped to the second; an effort that closed within that
    // second closed before it.
    let until = Timestamp::from_unix_ms((committed.unix_ms() / 1000 + 1) * 1000);
    let since = Timestamp::from_unix_ms(committed.unix_ms() - LOOKBACK_MS);
    for effort in svc.effort_store.list_in_window(since, until).await? {
        let closed_before = effort.ended_at.is_some_and(|at| at <= until);
        if closed_before && holds_effort(svc, ws, sha, &effort).await? {
            link(svc, sha, &effort).await?;
        }
    }
    Ok(())
}

/// Link `effort`, just finished, to each commit made since it started whose
/// work it is (the agent committed before the task closed).
pub async fn link_effort(svc: &crate::Services, effort: &EffortId) -> Result<(), DomainError> {
    let Some(effort) = svc.effort_store.get_effort(effort).await? else {
        return Ok(());
    };
    let Some(thread) = svc.thread_store.get(&effort.thread_id).await? else {
        return Ok(());
    };
    let ws = svc
        .worktrees
        .resolve(Some(&thread.stream_id.to_string()))
        .await;
    // To the second, as commits are stamped.
    let since = Timestamp::from_unix_ms(effort.started_at.unix_ms() / 1000 * 1000);
    for (sha, _) in svc.git_store.commits_since(since).await? {
        if holds_effort(svc, &ws, &sha, &effort).await? {
            link(svc, &sha, &effort).await?;
        }
    }
    Ok(())
}

/// Whether commit `sha` holds `effort`'s work ([`holds`] over the files
/// they share).
async fn holds_effort(
    svc: &crate::Services,
    ws: &Path,
    sha: &str,
    effort: &Effort,
) -> Result<bool, DomainError> {
    let Some(end) = effort.end_snapshot_id else {
        return Ok(false);
    };
    let effort_paths = svc.effort_store.paths(&effort.id).await?;
    let shared: Vec<String> = svc
        .git_store
        .commit_paths(sha)
        .await?
        .into_iter()
        .filter(|p| effort_paths.contains(p))
        .collect();
    let objects = svc.vcs.object_store(ws);
    let files = svc.snapshot_files();
    let mut pairs = Vec::with_capacity(shared.len());
    for path in &shared {
        let in_commit = svc
            .vcs
            .object_at(ws, sha, path)
            .await
            .map_err(|e| DomainError::Invalid(format!("{sha}:{path}: {e}")))?;
        let at_end = match files.read_file_at_snapshot(end, path).await {
            Ok(bytes) => bytes.map(|b| objects.id_of(&b)),
            // Its end version has aged out of Local History (or was never
            // kept): what it was can't be known, so it can't be shown to
            // hold the commit — and mustn't stop the commit linking to the
            // efforts that do (tsk1078).
            Err(SnapshotFileError::Expired | SnapshotFileError::NoContent) => return Ok(false),
            Err(e) => {
                return Err(DomainError::Invalid(format!("snapshot {end}:{path}: {e}")));
            }
        };
        pairs.push((in_commit, at_end));
    }
    Ok(holds(&pairs))
}

/// The `work_item → commit` edge saying `sha` holds `effort`'s work; none
/// while the effort is unlinked.
pub(crate) async fn link(
    svc: &crate::Services,
    sha: &str,
    effort: &Effort,
) -> Result<(), DomainError> {
    let Some(work_item) = effort
        .work_item
        .as_deref()
        .and_then(oxplow_domain::refs::build::work_item_id_of_ref)
    else {
        return Ok(());
    };
    let edge = PageRefEdge::new(KIND_WORK_ITEM, work_item, KIND_COMMIT, sha, RT_COMMITTED)
        .with_extra(serde_json::json!({ "effort": effort.id.to_string() }).to_string());
    svc.page_ref_store.upsert_edge(edge).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::test_fixtures::commit_all;

    /// The project's tasks per commit, as `v_commit_task` reads them.
    async fn tasks_of(svc: &crate::Services, sha: &str) -> serde_json::Value {
        let out = svc
            .sql
            .query_sql(
                "SELECT task_id FROM v_commit_task WHERE sha = ?1 ORDER BY task_id",
                vec![oxplow_db::SqlCell::Text(sha.to_string())],
                None,
            )
            .await
            .unwrap();
        serde_json::to_value(out.rows).unwrap()
    }

    /// `effort`, closed now: its end snapshot taken over the working tree,
    /// and `paths` as its files.
    async fn close_effort(svc: &crate::Services, effort: EffortId, paths: &[&str]) {
        let root = svc.layout.project_dir.clone();
        let capture = crate::snapshot_capture::SnapshotCaptureService::new(
            svc.snapshot_store.clone(),
            svc.blobs.clone(),
            root.clone(),
            Arc::new(crate::vcs::GitProvider),
            oxplow_domain::StreamId::new(1),
            1_000_000,
            oxplow_fs_watch::WorkspaceFilter::default(),
        )
        .with_settle_duration(std::time::Duration::ZERO)
        .with_predrain_delay(std::time::Duration::ZERO);
        for p in paths {
            capture.mark_dirty(root.join(p), oxplow_fs_watch::WatchEventKind::Other);
        }
        let end = capture
            .request_snapshot(crate::snapshot_capture::TakeRequest {
                trigger: oxplow_domain::snapshot::SnapshotTrigger::Manual,
                thread_id: None,
                turn_id: None,
                effort_id: None,
                budget: None,
            })
            .await
            .unwrap()
            .unwrap();
        svc.effort_store
            .set_end_snapshot(&effort, end)
            .await
            .unwrap();
        let (effort, paths): (i64, Vec<String>) = (
            effort.value(),
            paths.iter().map(|p| p.to_string()).collect(),
        );
        let ended = Timestamp::now().to_text();
        svc.db
            .transaction(move |c| {
                c.execute(
                    "UPDATE effort SET ended_at = ?2 WHERE id = ?1",
                    rusqlite::params![effort, ended],
                )
                .map_err(oxplow_db::map_sql_err)?;
                for p in &paths {
                    c.execute(
                        "INSERT INTO effort_file (effort_id, path, change_kind, vcs_rev_exact)
                         VALUES (?1, ?2, 'updated', 0)",
                        rusqlite::params![effort, p],
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                }
                Ok(())
            })
            .await
            .unwrap();
    }

    /// Index `sha` as the commit indexer does, then link it.
    async fn index_and_link(svc: &crate::Services, sha: &str) {
        let root = svc.layout.project_dir.clone();
        crate::commit_indexer::refresh(svc).await;
        let committed = svc
            .git_store
            .commits_since(Timestamp::from_unix_ms(0))
            .await
            .unwrap()
            .into_iter()
            .find(|(s, _)| s == sha)
            .map(|(_, at)| at)
            .expect("indexed");
        link_commit(svc, &root, sha, committed).await.unwrap();
    }

    #[test]
    fn a_commit_holds_work_when_every_shared_file_matches() {
        let id = |s: &str| Some(ObjectId(s.into()));
        assert!(holds(&[(id("a"), id("a"))]));
        assert!(holds(&[(None, None)]), "both deleted it");
        assert!(!holds(&[(id("a"), id("a")), (id("b"), id("c"))]));
        assert!(!holds(&[]), "nothing shared");
    }

    /// The walk's case: the agent commits exactly the effort's files after
    /// the task closed; a commit of other content doesn't link.
    #[tokio::test]
    async fn a_commit_of_an_efforts_work_links_its_task() {
        let f = crate::test_fixtures::services_with_task_effort().await;
        let root = f.svc.layout.project_dir.clone();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "fn a() {}\n").unwrap();
        commit_all(&root, "base");
        std::fs::write(root.join("src/a.rs"), "fn a() { 1 }\n").unwrap();
        close_effort(&f.svc, f.effort, &["src/a.rs", "src/c.rs"]).await;
        // Committed: the effort's version of the one file they share.
        let sha = commit_all(&root, "the discount fix");
        index_and_link(&f.svc, &sha).await;
        assert_eq!(
            tasks_of(&f.svc, &sha).await,
            serde_json::json!([[f.task.value()]])
        );
        // Edited again and committed: not the effort's version.
        std::fs::write(root.join("src/a.rs"), "fn a() { 2 }\n").unwrap();
        let other = commit_all(&root, "something else");
        index_and_link(&f.svc, &other).await;
        assert_eq!(tasks_of(&f.svc, &other).await, serde_json::json!([]));
    }

    /// tsk1078: an older effort whose end version has expired from Local
    /// History can't hold the commit, and mustn't stop it linking to the
    /// effort that does.
    #[tokio::test]
    async fn an_expired_effort_doesnt_stop_a_commit_linking() {
        let f = crate::test_fixtures::services_with_task_effort().await;
        let root = f.svc.layout.project_dir.clone();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "fn a() {}\n").unwrap();
        commit_all(&root, "base");
        std::fs::write(root.join("src/a.rs"), "fn a() { 1 }\n").unwrap();
        close_effort(&f.svc, f.effort, &["src/a.rs"]).await;
        // Its bytes age out.
        f.svc.blobs.gc(&Default::default()).unwrap();
        let later = f
            .svc
            .effort_store
            .start(
                &oxplow_domain::refs::build::work_item_ref(f.task),
                &f.thread,
                None,
            )
            .await
            .unwrap()
            .id;
        std::fs::write(root.join("src/a.rs"), "fn a() { 2 }\n").unwrap();
        close_effort(&f.svc, later, &["src/a.rs"]).await;
        let sha = commit_all(&root, "the second fix");
        index_and_link(&f.svc, &sha).await;
        assert_eq!(
            tasks_of(&f.svc, &sha).await,
            serde_json::json!([[f.task.value()]])
        );
    }

    /// Committed before the task closed: the effort's finish links it.
    #[tokio::test]
    async fn an_effort_finished_after_its_commit_links_it() {
        let f = crate::test_fixtures::services_with_task_effort().await;
        let root = f.svc.layout.project_dir.clone();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "fn a() { 1 }\n").unwrap();
        let sha = commit_all(&root, "the fix");
        crate::commit_indexer::refresh(&f.svc).await;
        close_effort(&f.svc, f.effort, &["src/a.rs"]).await;
        link_effort(&f.svc, &f.effort).await.unwrap();
        assert_eq!(
            tasks_of(&f.svc, &sha).await,
            serde_json::json!([[f.task.value()]])
        );
    }
}
