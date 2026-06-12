//! Cores for the `hooks` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use oxplow_app::agent_status_derive::derive_thread_status;
use oxplow_app::{HookEnvelope, Services};
use oxplow_domain::stores::AgentTurnStore;
use oxplow_domain::{AgentStatus, AgentTurn, HookEvent, HookKind, ThreadId};

use crate::error::IpcError;

/// Land an envelope from the hook subprocess. Drives the agent_turn /
/// agent_status state machine inside HookIngestService.
pub async fn ingest_hook_event(svc: &Services, envelope: HookEnvelope) -> Result<(), IpcError> {
    svc.hook_ingest
        .ingest(envelope)
        .await
        .map_err(|e| IpcError::internal(e.to_string()))?;
    Ok(())
}

pub async fn list_hook_events(
    svc: &Services,
    thread_id: Option<ThreadId>,
    limit: Option<usize>,
) -> Result<Vec<HookEvent>, IpcError> {
    let limit = limit.unwrap_or(200);
    Ok(svc
        .hook_event_store
        .list_recent(thread_id.as_ref(), limit)
        .await?)
}

pub async fn list_hook_events_by_kind(
    svc: &Services,
    kind: HookKind,
    limit: Option<usize>,
) -> Result<Vec<HookEvent>, IpcError> {
    Ok(svc
        .hook_event_store
        .list_by_kind(kind, limit.unwrap_or(200))
        .await?)
}

pub async fn list_agent_statuses(svc: &Services) -> Result<Vec<AgentStatus>, IpcError> {
    // Derive each thread's working/waiting state by replaying its
    // hook event log instead of trusting the agent_status row. The
    // sidecar table can drift (a missed Stop, a mis-routed Subagent
    // Stop, a stale boot row) — the hook log is what Claude Code
    // actually emitted, so deriving from it self-heals against
    // ingest-pipeline bugs. Mirrors `src/session/agent-status.ts`
    // on main, which has the proven state machine for this.
    let now = oxplow_domain::Timestamp::now();
    let mut statuses = svc.agent_status_store.list_all().await?;
    for s in &mut statuses {
        let events = svc
            .hook_event_store
            .list_recent(Some(&s.thread_id), 200)
            .await?;
        s.state = derive_thread_status(&events, now);
    }
    Ok(statuses)
}

pub async fn list_open_agent_turns(
    svc: &Services,
    thread_id: ThreadId,
) -> Result<Vec<AgentTurn>, IpcError> {
    Ok(svc.agent_turn_store.list_open(&thread_id).await?)
}

pub async fn list_recent_agent_turns(
    svc: &Services,
    thread_id: ThreadId,
    limit: Option<usize>,
) -> Result<Vec<AgentTurn>, IpcError> {
    Ok(svc
        .agent_turn_store
        .list_for_thread(&thread_id, limit.unwrap_or(50))
        .await?)
}

// Derivation logic + its unit tests live in
// oxplow_app::agent_status_derive — list_agent_statuses just wires
// the store calls together.

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn list_hook_events_dispatches_with_all_optionals_absent() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch("list_hook_events", serde_json::json!({}), &svc)
            .await
            .unwrap();
        assert!(out.is_array());
    }

    #[tokio::test]
    async fn list_agent_statuses_dispatches() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch("list_agent_statuses", serde_json::json!(null), &svc)
            .await
            .unwrap();
        assert!(out.is_array());
    }

    #[tokio::test]
    async fn agent_status_row_keeps_pane_and_state_semantics() {
        // Pins the DTO field semantics (audited after a live reading of
        // `{"pane_target":"working","state":"idle"}` looked like a
        // swap): `pane_target` is a pane NAME — "working" or "talking",
        // the tmux window the agent lives in (threads.pane_target
        // defaults to 'working' in the schema) — while `state` carries
        // the AgentStatusState enum. The two are never cross-assigned;
        // "the working pane's agent is idle" is a correct row.
        let (svc, _dir) = crate::test_support::services();
        crate::dispatch(
            "ingest_hook_event",
            serde_json::json!({
                "envelope": {
                    "kind": "user_prompt_submit",
                    "thread_id": "thr1",
                    "stream_id": null,
                    "session_id": "s1",
                    "payload_json": "{}",
                    "prompt": "do the thing",
                }
            }),
            &svc,
        )
        .await
        .unwrap();
        let out = crate::dispatch("list_agent_statuses", serde_json::json!(null), &svc)
            .await
            .unwrap();
        let rows = out.as_array().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["pane_target"], "working");
        assert_eq!(rows[0]["state"], "running");
    }
}
