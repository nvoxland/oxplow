//! Cores for the `hooks` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use oxplow_app::agent_status_derive::{derive_thread_status, recent_activity};
use oxplow_app::{HookEnvelope, Services};
use oxplow_domain::stores::AgentTurnStore;
use oxplow_domain::{AgentStatus, AgentTurn, StoredEvent, ThreadId};

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

/// The newest agent activity (`agent.*` events), newest first — on a
/// thread when given, else a stream, else everywhere. The activity log's
/// source (P3.9; it used to read an in-memory hook ring a restart
/// emptied). Bodies are behind [`read_event_content`].
pub async fn list_agent_events(
    svc: &Services,
    thread_id: Option<ThreadId>,
    stream_id: Option<oxplow_domain::StreamId>,
    limit: Option<usize>,
) -> Result<Vec<StoredEvent>, IpcError> {
    Ok(svc
        .event_log_store
        .recent("agent", thread_id, stream_id, limit.unwrap_or(200))
        .await?)
}

/// A stored event body (a tool's input or output), as text; `None` when
/// it was never stored or retention removed it.
pub async fn read_event_content(svc: &Services, hash: String) -> Result<Option<String>, IpcError> {
    Ok(oxplow_db::event_content_store::read(&svc.db, &hash)
        .await?
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()))
}

pub async fn list_agent_statuses(svc: &Services) -> Result<Vec<AgentStatus>, IpcError> {
    // Every thread that has logged a status, with its working/waiting
    // state derived by replaying its activity: a missed Stop or a dead
    // agent shows as what the log says happened (stalled), not as the
    // last status it announced.
    let now = oxplow_domain::Timestamp::now();
    let mut statuses = svc.agent_status_store.list_all().await?;
    for s in &mut statuses {
        let events = recent_activity(&svc.event_log_store, s.thread_id).await?;
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

/// One agent turn by id — the turn page's source for its start and end
/// snapshots (P2.11). `None` for an unknown id.
pub async fn get_agent_turn(
    svc: &Services,
    turn_id: oxplow_domain::AgentTurnId,
) -> Result<Option<AgentTurn>, IpcError> {
    use oxplow_domain::stores::AgentTurnStore as _;
    Ok(svc.agent_turn_store.get(&turn_id).await?)
}

// Derivation logic + its unit tests live in
// oxplow_app::agent_status_derive — list_agent_statuses just wires
// the store calls together.

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn list_agent_events_dispatches_with_all_optionals_absent() {
        let (svc, _dir) = crate::test_support::services();
        let out = crate::dispatch("list_agent_events", serde_json::json!({}), &svc)
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
    async fn a_thread_with_a_logged_status_is_listed_with_its_derived_state() {
        // The row list is every thread that has logged a status; the
        // state shown is derived from its activity.
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
        assert_eq!(rows[0]["thread_id"], "thr1");
        assert_eq!(rows[0]["state"], "running");
    }
}
