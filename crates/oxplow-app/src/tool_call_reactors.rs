//! Tool-call reactors (P3.5, tsk475): what a finished tool call records,
//! as consumers of `agent.tool.finished` on the event pump — checkpointed,
//! redelivered after a crash and dead-lettered like any other consumer,
//! instead of the best-effort writes the hook route used to make inline.
//!
//! - `tool_call.project` (sync) — the `agent_tool_call` row, a projection
//!   of the event (`v_tool_call`, `v_context_read`, `v_struggle`); one row
//!   per event however often it is delivered.
//! - `wiki.attribution` (sync) — an edit of `.oxplow/wiki/<slug>.md` marks
//!   the page touched by the thread (the rail's "Finished" list).
//! - `effort.claim` (async) — a structured edit (Edit / Write / MultiEdit /
//!   NotebookEdit) claims its file for the effort that was open when it
//!   happened (the event's effort anchor), falling back to target scoring
//!   when several were open. `Bash` / formatter writes stay for snapshot
//!   reconciliation.
//!
//! The ingest already made `path` relative to the thread's worktree, so
//! these read the payload as it is.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use oxplow_db::{Database, SqliteEffortStore};
use oxplow_domain::events::schema::{AgentToolFinished, EventType};
use oxplow_domain::{DomainError, StoredEvent, ThreadId};

use crate::event_pump::{AsyncEventConsumer, EventConsumer};
use crate::task_service::TaskService;

pub const TOOL_CALL_PROJECTION: &str = "tool_call.project";
pub const WIKI_ATTRIBUTION: &str = "wiki.attribution";
pub const EFFORT_CLAIM: &str = "effort.claim";

/// Tools that write the file they name.
fn is_structured_write(tool: &str) -> bool {
    matches!(tool, "Edit" | "Write" | "MultiEdit" | "NotebookEdit")
}

fn str_field<'a>(event: &'a StoredEvent, key: &str) -> Option<&'a str> {
    event.envelope.payload.get(key).and_then(|v| v.as_str())
}

/// Projects each `agent.tool.finished` into `agent_tool_call`.
pub struct ToolCallProjection;

impl EventConsumer for ToolCallProjection {
    fn name(&self) -> &'static str {
        TOOL_CALL_PROJECTION
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == AgentToolFinished::TYPE
    }

    fn handle(&self, conn: &rusqlite::Connection, event: &StoredEvent) -> Result<(), DomainError> {
        let env = &event.envelope;
        let Some(thread) = env.anchors.thread_id else {
            return Ok(()); // no thread, no row (the table requires one)
        };
        let call = oxplow_db::NewToolCall {
            thread_id: thread.value(),
            effort_id: env.anchors.effort_id.map(|e| e.value()),
            turn_id: env.anchors.turn_id,
            tool: str_field(event, "tool").unwrap_or_default().to_string(),
            path: str_field(event, "path").map(str::to_string),
            detail: str_field(event, "detail").map(str::to_string),
            ok: env.payload.get("ok").and_then(|v| v.as_bool()),
            event_id: Some(env.id.as_str().to_string()),
            at: Some(env.at),
        };
        oxplow_db::tool_call_store::record_tx(conn, &call).map(|_| ())
    }
}

/// The wiki-page slug for a path directly inside `.oxplow/wiki/` with a
/// `.md` extension — relative (to the worktree) or absolute in the
/// project.
pub fn wiki_slug(raw: &str, project_dir: &Path) -> Option<String> {
    let path = Path::new(raw);
    let rel = if path.is_absolute() {
        path.strip_prefix(project_dir).ok()?
    } else {
        path
    };
    let name = rel.strip_prefix(".oxplow/wiki").ok()?;
    if name.parent().is_some_and(|p| !p.as_os_str().is_empty()) {
        return None; // wiki pages are flat
    }
    let stem = name.file_stem()?.to_string_lossy().into_owned();
    (name.extension()? == "md").then_some(stem)
}

/// Marks a wiki page touched by the thread that edited it.
pub struct WikiAttribution {
    pub project_dir: PathBuf,
}

impl EventConsumer for WikiAttribution {
    fn name(&self) -> &'static str {
        WIKI_ATTRIBUTION
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == AgentToolFinished::TYPE
    }

    fn handle(&self, conn: &rusqlite::Connection, event: &StoredEvent) -> Result<(), DomainError> {
        let (Some(thread), Some(tool), Some(path)) = (
            event.envelope.anchors.thread_id,
            str_field(event, "tool"),
            str_field(event, "path"),
        ) else {
            return Ok(());
        };
        if !is_structured_write(tool) {
            return Ok(());
        }
        let Some(slug) = wiki_slug(path, &self.project_dir) else {
            return Ok(());
        };
        // Only an indexed page can be marked (the row references it). A page
        // the edit just created is indexed by the wiki watcher, which may
        // not have run yet; it goes unmarked, as it did before the reactor.
        let indexed: bool = conn
            .query_row(
                "SELECT EXISTS (SELECT 1 FROM wiki_page WHERE slug = ?1)",
                [&slug],
                |r| r.get(0),
            )
            .map_err(oxplow_db::map_sql_err)?;
        if !indexed {
            return Ok(());
        }
        oxplow_db::wiki_page_thread_updates::touch_tx(conn, thread, &slug, event.envelope.at)
    }
}

/// Claims an edited file for the effort it was edited in.
pub struct EffortClaimConsumer {
    tasks: TaskService,
    efforts: std::sync::Arc<SqliteEffortStore>,
    db: Database,
    project_dir: PathBuf,
}

impl EffortClaimConsumer {
    pub fn new(
        tasks: TaskService,
        efforts: std::sync::Arc<SqliteEffortStore>,
        db: Database,
        project_dir: PathBuf,
    ) -> Self {
        Self {
            tasks,
            efforts,
            db,
            project_dir,
        }
    }

    /// The thread's worktree (its stream's; the project dir when none).
    async fn worktree(&self, thread: ThreadId) -> Result<PathBuf, DomainError> {
        let wt = self
            .db
            .read(move |tx| {
                use rusqlite::OptionalExtension as _;
                let wt: Option<String> = tx
                    .query_row(
                        "SELECT s.worktree_path FROM threads th JOIN streams s ON s.id = th.stream_id
                          WHERE th.id = ?1",
                        [thread.value()],
                        |r| r.get(0),
                    )
                    .optional()
                    .map_err(oxplow_db::map_sql_err)?;
                Ok(wt)
            })
            .await?;
        Ok(wt
            .filter(|w| !w.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.project_dir.clone()))
    }
}

#[async_trait]
impl AsyncEventConsumer for EffortClaimConsumer {
    fn name(&self) -> &'static str {
        EFFORT_CLAIM
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == AgentToolFinished::TYPE
    }

    async fn handle(&self, event: &StoredEvent) -> Result<(), DomainError> {
        let (Some(thread), Some(tool), Some(path)) = (
            event.envelope.anchors.thread_id,
            str_field(event, "tool"),
            str_field(event, "path"),
        ) else {
            return Ok(());
        };
        // An absolute path is outside the thread's worktree: no effort's file.
        if !is_structured_write(tool) || Path::new(path).is_absolute() {
            return Ok(());
        }
        let worktree = self.worktree(thread).await?;
        self.tasks
            .claim_effort_file(
                &self.efforts,
                &thread,
                event.envelope.anchors.effort_id,
                path,
                Some(&worktree),
            )
            .await
            .map_or_else(claim_outcome, |_| Ok(()))
    }
}

/// A failed claim as the pump reads it: a storage error keeps its kind (a
/// `Busy` defers the event rather than parking it); a task that no longer
/// exists has nothing to claim.
fn claim_outcome(err: crate::task_service::TaskServiceError) -> Result<(), DomainError> {
    match err {
        crate::task_service::TaskServiceError::Storage(e) => Err(e),
        crate::task_service::TaskServiceError::NotFound(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_busy_claim_defers_and_a_vanished_task_is_nothing_to_claim() {
        use crate::task_service::TaskServiceError;
        let busy = claim_outcome(TaskServiceError::Storage(DomainError::Busy(
            "locked".into(),
        )));
        assert!(busy.unwrap_err().is_retryable());
        assert!(claim_outcome(TaskServiceError::NotFound(oxplow_domain::TaskId::new(1))).is_ok());
    }
    use crate::{HookEnvelope, ToolDecision};
    use oxplow_db::EffortStore as _;
    use oxplow_domain::HookKind;
    use serde_json::json;

    fn hook(
        thread: oxplow_domain::ThreadId,
        kind: HookKind,
        body: serde_json::Value,
    ) -> HookEnvelope {
        HookEnvelope {
            kind,
            thread_id: Some(thread),
            stream_id: None,
            session_id: Some("s".into()),
            payload_json: body.to_string(),
            prompt: Some("go".into()),
            decision: (kind == HookKind::PreToolUse).then_some(ToolDecision {
                allowed: true,
                reason: None,
            }),
        }
    }

    async fn rows(svc: &crate::Services, sql: &'static str) -> serde_json::Value {
        let sl = crate::sql_gateway::SqlGateway::new(svc.db.clone());
        serde_json::to_value(sl.query_sql(sql, vec![], None).await.unwrap().rows).unwrap()
    }

    /// The tool-call row is a projection of `agent.tool.finished`: anchored
    /// to its turn, and written once however often the event is delivered.
    #[tokio::test]
    async fn a_redelivered_tool_event_projects_one_row() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        svc.hook_ingest
            .ingest(hook(f.thread, HookKind::UserPromptSubmit, json!({})))
            .await
            .unwrap();
        svc.hook_ingest
            .ingest(hook(
                f.thread,
                HookKind::PostToolUse,
                json!({"tool_name": "Read", "tool_input": {"file_path": ".context/usability.md"}}),
            ))
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        // Rewind the projection's checkpoint: a redelivery.
        svc.event_log_store
            .set_checkpoint(TOOL_CALL_PROJECTION.into(), 0)
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        assert_eq!(
            rows(
                svc,
                "SELECT tool, path, turn_id IS NOT NULL FROM v_tool_call"
            )
            .await,
            json!([["Read", ".context/usability.md", 1]])
        );
        assert_eq!(
            rows(svc, "SELECT path FROM v_context_read").await,
            json!([[".context/usability.md"]])
        );
    }

    /// In a worktree stream (a sibling directory of the project) an edit's
    /// absolute path is inside the thread's worktree: it is claimed and
    /// recorded repo-relative all the same (tsk386).
    #[tokio::test]
    async fn worktree_stream_edits_are_claimed_and_recorded_relative() {
        let f = crate::test_fixtures::services_with_effort().await;
        let worktree = tempfile::tempdir().unwrap();
        let wt = worktree.path().to_string_lossy().to_string();
        f.svc
            .db
            .transaction(move |c| {
                c.execute(
                    "UPDATE streams SET worktree_path = ?1 WHERE id = 1",
                    [wt.as_str()],
                )
                .map_err(oxplow_db::map_sql_err)?;
                Ok(())
            })
            .await
            .unwrap();
        let file = worktree.path().join("src/a.rs");
        f.svc
            .hook_ingest
            .ingest(hook(
                f.thread,
                HookKind::PostToolUse,
                json!({"tool_name": "Edit", "tool_input": {"file_path": file.to_string_lossy()}, "tool_response": {}}),
            ))
            .await
            .unwrap();
        f.svc.event_pump.run_once().await.unwrap();
        assert_eq!(
            rows(&f.svc, "SELECT path FROM v_tool_call").await,
            json!([["src/a.rs"]])
        );
        assert_eq!(
            rows(&f.svc, "SELECT path FROM v_effort_file").await,
            json!([["src/a.rs"]])
        );
    }

    /// The claim goes to the effort that was open when the edit happened,
    /// even when the reactor runs after that effort closed.
    #[tokio::test]
    async fn an_edit_is_claimed_by_the_effort_it_happened_in() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        svc.hook_ingest
            .ingest(hook(
                f.thread,
                HookKind::PostToolUse,
                json!({"tool_name": "Write", "tool_input": {"file_path": "src/b.rs"}}),
            ))
            .await
            .unwrap();
        // The effort closes before the reactor gets to the edit.
        svc.effort_store
            .finish(&f.effort, None, None)
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        let files = svc.effort_store.list_files(&f.effort).await.unwrap();
        assert_eq!(
            files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            vec!["src/b.rs"]
        );
    }

    /// An edit of a wiki page records that the thread touched it; a page
    /// not indexed yet is skipped, not parked as a dead letter.
    #[tokio::test]
    async fn a_wiki_edit_is_attributed_to_the_thread() {
        let f = crate::test_fixtures::services_with_effort().await;
        f.svc
            .db
            .transaction(|c| {
                c.execute(
                    "INSERT INTO wiki_page (slug, title, body_path, created_at, updated_at)
                       VALUES ('architecture', 'A', '.oxplow/wiki/architecture.md', '2026-01-01', '2026-01-01')",
                    [],
                )
                .map_err(oxplow_db::map_sql_err)?;
                Ok(())
            })
            .await
            .unwrap();
        f.svc
            .hook_ingest
            .ingest(hook(
                f.thread,
                HookKind::PostToolUse,
                json!({"tool_name": "Write", "tool_input": {"file_path": ".oxplow/wiki/brand-new.md"}}),
            ))
            .await
            .unwrap();
        for _ in 0..2 {
            f.svc
                .hook_ingest
                .ingest(hook(
                    f.thread,
                    HookKind::PostToolUse,
                    json!({"tool_name": "Edit", "tool_input": {"file_path": ".oxplow/wiki/architecture.md"}}),
                ))
                .await
                .unwrap();
        }
        f.svc.event_pump.run_once().await.unwrap();
        let touched = f
            .svc
            .wiki_page_thread_updates
            .list_for_thread(&f.thread, 10)
            .await
            .unwrap();
        assert_eq!(
            touched.iter().map(|t| t.slug.as_str()).collect::<Vec<_>>(),
            vec!["architecture"]
        );
        assert!(f
            .svc
            .event_pump
            .list_dead_letters(true)
            .await
            .unwrap()
            .is_empty());
    }

    #[test]
    fn wiki_slugs_are_flat_md_files_under_the_wiki_dir() {
        let project = std::path::Path::new("/p");
        assert_eq!(
            wiki_slug(".oxplow/wiki/data-model.md", project).as_deref(),
            Some("data-model")
        );
        assert_eq!(
            wiki_slug("/p/.oxplow/wiki/x.md", project).as_deref(),
            Some("x")
        );
        assert_eq!(wiki_slug(".oxplow/wiki/sub/inner.md", project), None);
        assert_eq!(wiki_slug(".oxplow/wiki/foo.txt", project), None);
        assert_eq!(wiki_slug("README.md", project), None);
        assert_eq!(wiki_slug("/etc/hosts", project), None);
    }
}
