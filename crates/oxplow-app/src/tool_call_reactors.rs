//! Tool-call reactors (P3.5): what a finished tool call records,
//! as consumers of `agent.tool.finished` on the event pump — checkpointed,
//! redelivered after a crash and dead-lettered like any other consumer,
//! instead of the best-effort writes the hook route used to make inline.
//!
//! - `tool_call.project` (sync) — the `agent_tool_call` row, a projection
//!   of a call's events (`v_tool_call`, `v_context_read`): it opens at the
//!   `agent.tool.requested` (running, or denied), closes at the
//!   `agent.tool.finished` its request caused, and is abandoned when its
//!   turn's `agent.turn.ended` (or its session's `agent.session.ended`)
//!   comes with it still running. One row per
//!   call however often an event is delivered.
//!   Every file a call names, or its harness reports it changed, is its
//!   `agent_tool_call_path` row (`v_tool_call_path`).
//!
//! The ingest already made `paths` relative to the thread's worktree, so
//! these read the payload as it is.

use oxplow_db::tool_call_store::CallOwner;
use oxplow_domain::events::schema::{
    AgentSessionEnded, AgentToolFinished, AgentToolRequested, AgentTurnEnded, EventType,
};
use oxplow_domain::{DomainError, StoredEvent};

use crate::event_pump::EventConsumer;

pub const TOOL_CALL_PROJECTION: &str = "tool_call.project";

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

/// Projects a call's events into its `agent_tool_call` row.
pub struct ToolCallProjection;

impl EventConsumer for ToolCallProjection {
    fn name(&self) -> &'static str {
        TOOL_CALL_PROJECTION
    }

    fn handles(&self, event_type: &str) -> bool {
        [
            AgentToolRequested::TYPE,
            AgentToolFinished::TYPE,
            AgentTurnEnded::TYPE,
            AgentSessionEnded::TYPE,
        ]
        .contains(&event_type)
    }

    fn handle(&self, conn: &rusqlite::Connection, event: &StoredEvent) -> Result<(), DomainError> {
        let env = &event.envelope;
        // What a turn or a session left running when it ended is abandoned.
        let owner = match env.event_type.as_str() {
            t if t == AgentTurnEnded::TYPE => env.anchors.turn_id.map(CallOwner::Turn),
            t if t == AgentSessionEnded::TYPE => env
                .anchors
                .agent_session_id
                .map(|s| CallOwner::Session(s.value())),
            _ => None,
        };
        if env.event_type == AgentTurnEnded::TYPE || env.event_type == AgentSessionEnded::TYPE {
            if let Some(owner) = owner {
                oxplow_db::tool_call_store::abandon_running_tx(conn, owner, env.at)?;
            }
            return Ok(());
        }
        let Some(thread) = env.anchors.thread_id else {
            return Ok(()); // no thread, no row (the table requires one)
        };
        let finished = env.event_type == AgentToolFinished::TYPE;
        let call = oxplow_db::NewToolCall {
            thread_id: thread.value(),
            turn_id: env.anchors.turn_id,
            tool: str_field(event, "tool").unwrap_or_default().to_string(),
            kind: str_field(event, "kind").unwrap_or("other").to_string(),
            // A row per call: its first file.
            path: paths(event).first().map(|p| p.to_string()),
            detail: str_field(event, "detail").map(str::to_string),
            ok: env.payload.get("ok").and_then(|v| v.as_bool()),
            event_id: finished.then(|| env.id.as_str().to_string()),
            at: Some(env.at),
            agent_session_id: env.anchors.agent_session_id.map(|s| s.value()),
            // Absent before the harness reports a subagent, so optional.
            subagent_id: env.payload["subagent"]["id"].as_str().map(str::to_string),
            subagent_kind: env.payload["subagent"]["kind"].as_str().map(str::to_string),
            call_id: str_field(event, "call").map(str::to_string),
            duration_ms: env.payload.get("duration_ms").and_then(|v| v.as_u64()),
        };
        if finished {
            // Its request is its cause, when one was logged.
            oxplow_db::tool_call_store::record_finish_tx(
                conn,
                &call,
                env.cause.as_ref().map(|c| c.as_str()),
            )?;
            // The files it changed, every one, as its finish reports them.
            oxplow_db::tool_call_store::record_paths_tx(
                conn,
                env.id.as_str(),
                &call.kind,
                &paths(event),
            )
        } else {
            let denied = str_field(event, "decision") == Some("denied");
            oxplow_db::tool_call_store::record_request_tx(conn, &call, env.id.as_str(), denied)
                .map(|_| ())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::{HookEnvelope, ToolDecision};
    use oxplow_domain::HookKind;
    use serde_json::json;

    fn hook(
        thread: oxplow_domain::ThreadId,
        kind: HookKind,
        body: serde_json::Value,
    ) -> HookEnvelope {
        HookEnvelope {
            session_title: None,
            turn_run: None,
            reading: Default::default(),
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
            harness_turn: None,
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
        let f = crate::test_fixtures::services_with_thread().await;
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
        let f = crate::test_fixtures::services_with_thread().await;
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
    /// absolute path is inside the thread's worktree: it is recorded
    /// repo-relative all the same.
    #[tokio::test]
    async fn worktree_stream_edits_are_recorded_relative() {
        let f = crate::test_fixtures::services_with_thread().await;
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
            rows(&f.svc, "SELECT path FROM v_tool_call_path").await,
            json!([["src/a.rs"]])
        );
    }

    /// A shell call's files the harness reported (`bashEditDiff`) are on its
    /// tool call with the basis `reported`; one that reported none names
    /// nothing; a file an edit tool named is `named`.
    #[tokio::test]
    async fn a_shell_calls_reported_files_are_recorded_apart_from_named_ones() {
        let f = crate::test_fixtures::services_with_thread().await;
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
        let abs = |p: &str| worktree.path().join(p).to_string_lossy().to_string();
        let shell = |id: &str, response: serde_json::Value| {
            json!({"tool_name": "Bash", "tool_input": {"command": "make"}, "tool_use_id": id,
                   "tool_response": response})
        };
        let ingest = |body| {
            f.svc
                .hook_ingest
                .ingest(hook(f.thread, HookKind::PostToolUse, body))
        };
        // Named by an edit tool first.
        ingest(json!({"tool_name": "Edit", "tool_input": {"file_path": abs("named.rs")}, "tool_response": {}}))
            .await
            .unwrap();
        // A shell call reporting two files (one already named) and one not.
        ingest(shell(
            "t1",
            json!({"stdout": "", "bashEditDiff": {
                "files": [{"filePath": abs("gen.rs"), "created": true}],
                "changedFiles": [abs("gen.rs"), abs("named.rs")], "moreFiles": 0}}),
        ))
        .await
        .unwrap();
        ingest(shell("t2", json!({"stdout": "built"})))
            .await
            .unwrap();
        f.svc.event_pump.run_once().await.unwrap();
        assert_eq!(
            rows(
                &f.svc,
                "SELECT call_id, path FROM v_tool_call WHERE tool = 'Bash' ORDER BY call_id"
            )
            .await,
            json!([["t1", "gen.rs"], ["t2", null]])
        );
        assert_eq!(
            rows(
                &f.svc,
                "SELECT path, min(basis) FROM v_tool_call_path GROUP BY path ORDER BY path"
            )
            .await,
            json!([["gen.rs", "reported"], ["named.rs", "named"]])
        );
    }

    /// A turn that ends because its agent went away (Codex's daemon
    /// restarting under it: an `Interrupt`, not a `Stop`) abandons the call it
    /// left running too: a call whose `item/completed` can never come is not
    /// `running` for ever.
    #[tokio::test]
    async fn an_interrupted_turn_abandons_the_call_in_flight() {
        let f = crate::test_fixtures::services_with_thread().await;
        let svc = &f.svc;
        let ingest = |h: HookEnvelope| svc.hook_ingest.ingest(h);
        ingest(hook(f.thread, HookKind::UserPromptSubmit, json!({})))
            .await
            .unwrap();
        ingest(hook(
            f.thread,
            HookKind::PreToolUse,
            json!({"tool_name": "Bash", "tool_input": {"command": "sleep 40"}, "tool_use_id": "t1"}),
        ))
        .await
        .unwrap();
        ingest(hook(f.thread, HookKind::Interrupt, json!({})))
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        assert_eq!(
            rows(
                svc,
                "SELECT call_id, state, ended_at IS NOT NULL FROM v_tool_call"
            )
            .await,
            json!([["t1", "abandoned", 1]])
        );
    }

    /// A call's row opens at its request and closes at its finish, with the
    /// harness's timing; a denied call has a row that never ran; a call its
    /// turn ended with is abandoned; a finish with no request still records.
    #[tokio::test]
    async fn a_tool_calls_row_follows_it_from_request_to_finish() {
        let f = crate::test_fixtures::services_with_thread().await;
        let svc = &f.svc;
        let ingest = |h: HookEnvelope| svc.hook_ingest.ingest(h);
        let call = |id: &str, cmd: &str| json!({"tool_name": "Bash", "tool_input": {"command": cmd}, "tool_use_id": id});
        ingest(hook(f.thread, HookKind::UserPromptSubmit, json!({})))
            .await
            .unwrap();
        ingest(hook(
            f.thread,
            HookKind::PreToolUse,
            call("t1", "cargo build"),
        ))
        .await
        .unwrap();
        svc.event_pump.run_once().await.unwrap();
        assert_eq!(
            rows(
                svc,
                "SELECT call_id, state, started_at IS NOT NULL, ended_at FROM v_tool_call"
            )
            .await,
            json!([["t1", "running", 1, null]])
        );
        let mut done = call("t1", "cargo build");
        done["duration_ms"] = json!(4200);
        ingest(hook(f.thread, HookKind::PostToolUse, done))
            .await
            .unwrap();
        let mut denied = hook(f.thread, HookKind::PreToolUse, call("t2", "rm -rf /"));
        denied.decision = Some(ToolDecision {
            allowed: false,
            reason: Some("no".into()),
        });
        ingest(denied).await.unwrap();
        ingest(hook(f.thread, HookKind::PostToolUse, call("t3", "ls")))
            .await
            .unwrap();
        ingest(hook(f.thread, HookKind::PreToolUse, call("t4", "sleep 60")))
            .await
            .unwrap();
        ingest(hook(
            f.thread,
            HookKind::Stop,
            json!({"last_assistant_message": "ok"}),
        ))
        .await
        .unwrap();
        svc.event_pump.run_once().await.unwrap();
        assert_eq!(
            rows(
                svc,
                "SELECT call_id, state, started_at IS NOT NULL, ended_at IS NOT NULL, duration_ms,
                        event_id IS NOT NULL
                   FROM v_tool_call ORDER BY id"
            )
            .await,
            json!([
                ["t1", "finished", 1, 1, 4200, 1],
                ["t2", "denied", 1, 1, null, 0],
                ["t3", "finished", 0, 1, null, 1],
                ["t4", "abandoned", 1, 1, null, 0],
            ])
        );
        // A redelivery changes nothing.
        svc.event_log_store
            .set_checkpoint(TOOL_CALL_PROJECTION.into(), 0)
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        assert_eq!(
            rows(svc, "SELECT count(*) FROM v_tool_call").await,
            json!([[4]])
        );
    }
}
