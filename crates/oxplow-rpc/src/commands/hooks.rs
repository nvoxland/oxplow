//! Cores for the `hooks` command module. Populated by the
//! oxplow-tauri-ipc -> oxplow-rpc migration; see crate docs.

use oxplow_app::agent_status_derive::{derive_session_status, recent_activity};
use oxplow_app::{HookEnvelope, Services};
use oxplow_domain::AgentStatus;

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

/// One body of an event (a tool's input or output), capped; `None` when it
/// was never stored or retention removed it. The person's read: any stream.
pub async fn read_event_content(
    svc: &Services,
    event_id: String,
    body: oxplow_app::event_bodies::EventBodyKey,
) -> Result<Option<oxplow_app::event_bodies::EventBody>, IpcError> {
    use oxplow_app::event_bodies::{read, EventBodyError};
    read(svc, &event_id, body, None).await.map_err(|e| match e {
        EventBodyError::Storage(e) => e.into(),
        other => IpcError::invalid(other.to_string()),
    })
}

pub async fn list_agent_statuses(svc: &Services) -> Result<Vec<AgentStatus>, IpcError> {
    // Every open agent session that has logged a status (and each
    // thread's activity no session claims), with its working/waiting state
    // derived by replaying its activity: a missed Stop or a dead agent
    // shows as what the log says happened (stalled), not as the last
    // status it announced. The renderer rolls a thread's sessions up.
    let now = oxplow_domain::Timestamp::now();
    let mut statuses = svc.agent_status_store.list_all().await?;
    for s in &mut statuses {
        let events = recent_activity(&svc.event_log_store, s.thread_id, s.agent_session_id).await?;
        s.state = derive_session_status(&events, now);
    }
    Ok(statuses)
}

// Derivation logic + its unit tests live in
// oxplow_app::agent_status_derive — list_agent_statuses just wires
// the store calls together.

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn list_agent_statuses_dispatches() {
        let (svc, _dir) = crate::test_support::services().await;
        let out = crate::dispatch("list_agent_statuses", serde_json::json!(null), &svc)
            .await
            .unwrap();
        assert!(out.is_array());
    }

    #[tokio::test]
    async fn a_thread_with_a_logged_status_is_listed_with_its_derived_state() {
        // The row list is every thread that has logged a status; the
        // state shown is derived from its activity.
        let (svc, _dir) = crate::test_support::services().await;
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
