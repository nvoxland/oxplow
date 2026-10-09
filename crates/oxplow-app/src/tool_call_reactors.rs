//! Tool-call reactors (P3.5, tsk475): what a finished tool call records,
//! as consumers of `agent.tool.finished` on the event pump — checkpointed,
//! redelivered after a crash and dead-lettered like any other consumer,
//! instead of the best-effort writes the hook route used to make inline.
//!
//! - `tool_call.project` (sync) — the `agent_tool_call` row, a projection
//!   of the event (`v_tool_call`, `v_context_read`, `v_struggle`); one row
//!   per event however often it is delivered.
//! - `effort.claim` (async) — an edit (`kind: edit`, whatever its harness
//!   calls it) claims every file it names for the effort that was open
//!   when it happened (the event's effort anchor), falling back to target
//!   scoring when several were open. Shell and formatter writes stay for
//!   snapshot reconciliation.
//!
//! The ingest already made `paths` relative to the thread's worktree, so
//! these read the payload as it is.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use oxplow_db::Database;
use oxplow_domain::events::schema::{AgentToolFinished, EventType};
use oxplow_domain::{DomainError, StoredEvent, ThreadId};

use crate::effort_service::EffortService;
use crate::event_pump::{AsyncEventConsumer, EventConsumer};

pub const TOOL_CALL_PROJECTION: &str = "tool_call.project";
pub const EFFORT_CLAIM: &str = "effort.claim";

fn str_field<'a>(event: &'a StoredEvent, key: &str) -> Option<&'a str> {
    event.envelope.payload.get(key).and_then(|v| v.as_str())
}

/// The files the call names (`paths`).
fn paths(event: &StoredEvent) -> Vec<&str> {
    event.envelope.payload["paths"]
        .as_array()
        .map(|a| a.iter().filter_map(|p| p.as_str()).collect())
        .unwrap_or_default()
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
            kind: str_field(event, "kind").unwrap_or("other").to_string(),
            // A row per call: its first file.
            path: paths(event).first().map(|p| p.to_string()),
            detail: str_field(event, "detail").map(str::to_string),
            ok: env.payload.get("ok").and_then(|v| v.as_bool()),
            event_id: Some(env.id.as_str().to_string()),
            at: Some(env.at),
            agent_session_id: env.anchors.agent_session_id.map(|s| s.value()),
            // Absent before the harness reports a subagent, so optional.
            subagent_id: env.payload["subagent"]["id"].as_str().map(str::to_string),
            subagent_kind: env.payload["subagent"]["kind"].as_str().map(str::to_string),
        };
        oxplow_db::tool_call_store::record_tx(conn, &call).map(|_| ())
    }
}

/// Claims an edited file for the effort it was edited in.
pub struct EffortClaimConsumer {
    efforts: EffortService,
    db: Database,
    project_dir: PathBuf,
}

impl EffortClaimConsumer {
    pub fn new(efforts: EffortService, db: Database, project_dir: PathBuf) -> Self {
        Self {
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
        let Some(thread) = event.envelope.anchors.thread_id else {
            return Ok(());
        };
        if str_field(event, "kind") != Some("edit") {
            return Ok(());
        }
        // An absolute path is outside the thread's worktree: no effort's file.
        let files: Vec<&str> = paths(event)
            .into_iter()
            .filter(|p| !Path::new(p).is_absolute())
            .collect();
        if files.is_empty() {
            return Ok(());
        }
        let worktree = self.worktree(thread).await?;
        for path in files {
            self.efforts
                .claim_effort_file(
                    &thread,
                    event.envelope.anchors.effort_id,
                    event.envelope.anchors.agent_session_id,
                    path,
                    Some(&worktree),
                )
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            agent_session_id: None,
            session_id: Some("s".into()),
            payload_json: body.to_string(),
            prompt: Some("go".into()),
            decision: (kind == HookKind::PreToolUse).then_some(ToolDecision {
                allowed: true,
                reason: None,
            }),
            tool: crate::test_fixtures::claude_tool(kind, &body),
            subagent: None,
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

    /// The row carries the session whose call it was, from the event's
    /// anchors.
    #[tokio::test]
    async fn the_projected_row_carries_the_session() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        let thread = f.thread.value();
        svc.db
            .transaction(move |c| {
                c.execute(
                    "INSERT INTO agent_session (id, thread_id, kind, harness, opened_at, updated_at)
                     VALUES (9, ?1, 'terminal', 'claude', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
                    [thread],
                )
                .map_err(oxplow_db::map_sql_err)?;
                Ok(())
            })
            .await
            .unwrap();
        let session = Some(oxplow_domain::AgentSessionId::new(9));
        for (kind, body) in [
            (HookKind::UserPromptSubmit, json!({})),
            (
                HookKind::PostToolUse,
                json!({"tool_name": "Read", "tool_input": {"file_path": "a.rs"}}),
            ),
        ] {
            let mut h = hook(f.thread, kind, body);
            h.agent_session_id = session;
            svc.hook_ingest.ingest(h).await.unwrap();
        }
        svc.event_pump.run_once().await.unwrap();
        assert_eq!(
            rows(svc, "SELECT agent_session_id, subagent_id FROM v_tool_call").await,
            json!([[9, null]])
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
}
