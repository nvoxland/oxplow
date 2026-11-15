//! A commit is linked to the requests that made it, per path.
//!
//! On `vcs.commit.indexed` (the commit indexer, as a commit is stored), for
//! each path the commit changed: the requests on the commit's stream that
//! changed it since the previous commit that held it, where the file as the
//! work left it is the commit's content (`commit_request`,
//! `oxplow_db::commit_request_store`). A snapshot row keeps the git blob id
//! of its bytes, so the match reads no bytes and survives Local History
//! expiry. Nothing is closed by a commit: the
//! `task → commit` edge is derived through the requests' tasks
//! (`v_oxplow_request_link`, `v_commit_task`).

use std::path::Path;

use oxplow_db::commit_request_store::{self, CommitPath};
use oxplow_domain::{DomainError, Timestamp};

/// Link commit `sha` (just indexed from workspace `ws`, committed at
/// `committed`) to the requests whose changes it holds. A workspace that is
/// no stream's, or a commit that changed no path, links nothing.
pub async fn link_commit(
    svc: &crate::Services,
    ws: &Path,
    sha: &str,
    committed: Timestamp,
) -> Result<(), DomainError> {
    let worktree = ws.to_string_lossy().to_string();
    let Some(stream) = svc
        .db
        .read(move |c| commit_request_store::stream_of_worktree_tx(c, &worktree))
        .await?
    else {
        return Ok(());
    };
    let changed = svc.git_store.commit_paths(sha).await?;
    let mut blobs: Vec<(String, Option<String>)> = Vec::with_capacity(changed.len());
    for path in changed {
        let blob = svc
            .vcs
            .object_at(ws, sha, &path)
            .await
            .map_err(|e| DomainError::Invalid(format!("{sha}:{path}: {e}")))?
            .map(|id| id.0);
        blobs.push((path, blob));
    }
    let sha = sha.to_string();
    svc.db
        .transaction(move |tx| {
            let paths: Vec<CommitPath<'_>> = blobs
                .iter()
                .map(|(path, blob)| CommitPath {
                    path,
                    blob: blob.as_deref(),
                })
                .collect();
            commit_request_store::link_commit_tx(
                tx,
                stream,
                &sha,
                committed,
                &paths,
                Timestamp::now(),
            )
            .map(|_| ())
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_domain::HookKind;
    use serde_json::json;

    use crate::test_fixtures::commit_all;

    fn hook(
        fx: &crate::test_fixtures::ThreadFixture,
        kind: HookKind,
        prompt: Option<&str>,
        body: serde_json::Value,
    ) -> crate::HookEnvelope {
        crate::HookEnvelope {
            session_title: None,
            turn_run: None,
            reading: Default::default(),
            kind,
            thread_id: Some(fx.thread),
            stream_id: None,
            agent_session_id: Some(fx.session),
            session_id: Some("s".into()),
            payload_json: body.to_string(),
            prompt: prompt.map(Into::into),
            decision: None,
            tool: crate::test_fixtures::claude_tool(kind, &body),
            subagent: None,
            harness_turn: None,
        }
    }

    /// A snapshot take over the working tree, as the capture service takes
    /// it (its rows carry the git blob id of what they captured).
    async fn capture(svc: &crate::Services, paths: &[&str]) {
        let root = svc.layout.project_dir.clone();
        let capture = crate::snapshot_capture::SnapshotCaptureService::new(
            svc.snapshot_store.clone(),
            svc.blobs.clone(),
            root.clone(),
            std::sync::Arc::new(crate::vcs::GitProvider),
            oxplow_domain::StreamId::new(1),
            1_000_000,
            oxplow_fs_watch::WorkspaceFilter::default(),
        )
        .with_settle_duration(std::time::Duration::ZERO)
        .with_predrain_delay(std::time::Duration::ZERO);
        for p in paths {
            capture.mark_dirty(root.join(p), oxplow_fs_watch::WatchEventKind::Other);
        }
        capture
            .request_snapshot(crate::snapshot_capture::TakeRequest {
                trigger: oxplow_domain::snapshot::SnapshotTrigger::Manual,
                thread_id: None,
                turn_id: None,
                budget: None,
                provider: None,
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

    async fn links(svc: &crate::Services, sha: &str) -> Vec<(String, String, String)> {
        let sha = sha.to_string();
        svc.db
            .read(move |c| {
                let mut stmt = c
                    .prepare(
                        "SELECT path, basis, confidence FROM commit_request WHERE sha = ?1
                          ORDER BY path",
                    )
                    .map_err(oxplow_db::map_sql_err)?;
                let rows = stmt
                    .query_map([sha], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                    .map_err(oxplow_db::map_sql_err)?;
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap()
    }

    /// The end to end case: a turn changes a file and the work is captured
    /// (the snapshot row keeps the git blob id of the bytes it captured); a
    /// commit of exactly that content links the request; a commit made outside oxplow, of other content, links nothing.
    #[tokio::test]
    async fn a_commit_of_a_turns_file_links_its_request() {
        let f = crate::test_fixtures::services_with_thread().await;
        let svc = &f.svc;
        let root = svc.layout.project_dir.clone();
        std::fs::create_dir_all(root.join("src")).unwrap();
        crate::test_fixtures::write_settings_fixture(root.join("src/a.rs"), "fn a() {}\n").unwrap();
        commit_all(&root, "base");
        capture(svc, &["src/a.rs"]).await;

        svc.hook_ingest
            .ingest(hook(
                &f,
                HookKind::UserPromptSubmit,
                Some("fix a"),
                json!({}),
            ))
            .await
            .unwrap();
        crate::test_fixtures::write_settings_fixture(root.join("src/a.rs"), "fn a() { 1 }\n")
            .unwrap();
        capture(svc, &["src/a.rs"]).await;
        svc.hook_ingest
            .ingest(hook(
                &f,
                HookKind::PostToolUse,
                None,
                json!({"tool_name": "Edit", "tool_use_id": "t1",
                       "tool_input": {"file_path": "src/a.rs"}, "tool_response": {}}),
            ))
            .await
            .unwrap();
        svc.hook_ingest
            .ingest(hook(&f, HookKind::Stop, None, json!({})))
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();

        let sha = commit_all(&root, "the fix");
        index_and_link(svc, &sha).await;
        assert_eq!(
            links(svc, &sha).await,
            vec![(
                "src/a.rs".to_string(),
                "named".to_string(),
                "sole".to_string()
            )]
        );
        // Edited outside oxplow and committed: no snapshot row holds it.
        crate::test_fixtures::write_settings_fixture(root.join("src/a.rs"), "fn a() { 2 }\n")
            .unwrap();
        let other = commit_all(&root, "something else");
        index_and_link(svc, &other).await;
        assert_eq!(links(svc, &other).await, vec![]);
    }
}
