//! Keeps `v_effort_metric_delta` / `v_effort_observation` current. The
//! metric engine computes an effort's deltas and evidence; lenses can't, so
//! core stores them: when an effort closes, and (debounced) for open efforts
//! as metric data or observations arrive. Emits `EffortEvidenceChanged`,
//! which it doesn't listen to, so it can't loop. See
//! `.context/semantic-layer.md`.

use std::sync::Arc;
use std::time::Duration;

use crate::OxplowEvent;

/// Quiet period before recomputing open efforts after data arrives.
pub const DEBOUNCE: Duration = Duration::from_secs(3);

/// Whether `event` means open efforts' evidence may have changed.
pub fn affects_open_efforts(event: &OxplowEvent) -> bool {
    matches!(
        event,
        OxplowEvent::MetricSamplesChanged { .. }
            | OxplowEvent::EffortObservationsChanged { .. }
            | OxplowEvent::AgentTokenUsageChanged { .. }
    )
}

/// Recompute one effort's evidence and announce it; failures are logged.
async fn refresh(state: &crate::Services, effort_id: i64) {
    let id = oxplow_domain::EffortId::new(effort_id).to_string();
    match state
        .collection
        .refresh_effort_evidence(&id, &state.effort_evidence_store)
        .await
    {
        Ok(()) => state
            .events
            .emit(OxplowEvent::EffortEvidenceChanged { effort_id }),
        Err(error) => tracing::warn!(effort_id, %error, "refreshing effort evidence failed"),
    }
}

/// Background: refresh an effort when it closes, and open efforts
/// (debounced) when their data may have changed.
pub fn spawn(state: Arc<crate::Services>) {
    let mut rx = state.events.subscribe();
    let (dirty_tx, mut dirty_rx) = tokio::sync::mpsc::unbounded_channel::<()>();
    let debounced = state.clone();
    tokio::spawn(async move {
        while dirty_rx.recv().await.is_some() {
            // Coalesce a burst of data events into one pass.
            loop {
                match tokio::time::timeout(DEBOUNCE, dirty_rx.recv()).await {
                    Ok(Some(())) => continue,
                    Ok(None) => return,
                    Err(_) => break,
                }
            }
            match debounced.effort_store.list_all_open().await {
                Ok(open) => {
                    for e in open {
                        refresh(&debounced, e.id.value()).await;
                    }
                }
                Err(error) => tracing::warn!(%error, "listing open efforts failed"),
            }
        }
    });
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(OxplowEvent::EffortFinished { effort_id, .. }) => {
                    let state = state.clone();
                    tokio::spawn(async move { refresh(&state, effort_id).await });
                }
                Ok(ev) if affects_open_efforts(&ev) => {
                    let _ = dirty_tx.send(());
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let _ = dirty_tx.send(());
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_data_events_trigger_a_refresh() {
        assert!(affects_open_efforts(&OxplowEvent::MetricSamplesChanged {
            stream_id: oxplow_domain::StreamId::new(1),
            measures: vec![],
        }));
        assert!(affects_open_efforts(
            &OxplowEvent::EffortObservationsChanged {
                thread_id: oxplow_domain::ThreadId::new(1),
                effort_id: "eff1".into(),
            }
        ));
        assert!(!affects_open_efforts(&OxplowEvent::EffortEvidenceChanged {
            effort_id: 1
        }));
        assert!(!affects_open_efforts(&OxplowEvent::PageVisitChanged));
    }

    #[tokio::test]
    async fn closing_an_effort_stores_its_evidence_and_announces_it() {
        let f = crate::test_fixtures::services_with_effort().await;
        let mut rx = f.svc.events.subscribe();
        spawn(f.svc.clone());
        tokio::task::yield_now().await;
        f.svc
            .tasks
            .update(
                f.task,
                crate::task_service::UpdateTaskChanges {
                    status: Some(oxplow_domain::TaskStatus::Done),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let want = f.effort.value();
        let got = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Ok(OxplowEvent::EffortEvidenceChanged { effort_id }) = rx.recv().await {
                    if effort_id == want {
                        return effort_id;
                    }
                }
            }
        })
        .await
        .expect("EffortEvidenceChanged after the effort closed");
        assert_eq!(got, want);
    }
}
