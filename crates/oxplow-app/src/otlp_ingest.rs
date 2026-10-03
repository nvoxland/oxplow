//! An agent's token telemetry in (P10.M2, `.context/agent-model.md` →
//! "Token usage"): the control plane's OTLP receiver (`POST /v1/metrics`,
//! `/v1/logs`) hands an export here. It is decoded and logged as one
//! `agent.tokens.reported` event in one transaction — anchored to the turn
//! the export measured (exports arrive after it) and the thread's single
//! open effort — and the `token_usage.otlp` consumer turns the event into
//! token facts. Nothing else writes them.

use std::sync::Arc;

use oxplow_db::agent_stores::{activity_anchors_tx, turn_at_tx};
use oxplow_db::event_log_store::{append_unique_tx, EventCtx};
use oxplow_db::Database;
use oxplow_domain::events::schema::{AgentTokensReported, AgentTokensReportedV1, TokenCount};
use oxplow_domain::refs::build::{thread_ref, turn_ref};
use oxplow_domain::vocabulary::VocabularyHandle;
use oxplow_domain::{DomainError, ThreadId};

use crate::event_pump::EventPump;
use crate::otlp_tokens::decode_token_export;

/// Logs agents' token exports as events.
#[derive(Clone)]
pub struct OtlpIngestService {
    db: Database,
    vocabulary: VocabularyHandle,
    pump: Arc<EventPump>,
}

impl OtlpIngestService {
    pub fn new(db: Database, vocabulary: VocabularyHandle, pump: Arc<EventPump>) -> Self {
        Self {
            db,
            vocabulary,
            pump,
        }
    }

    /// Log `thread`'s export `body` as `agent.tokens.reported`. Whether it
    /// logged one: a body with no token counts (most Codex log events)
    /// logs nothing, nor does a retransmit of an export already logged
    /// (keyed by the body's hash — each export carries its own window's
    /// timestamps, so two exports never hash alike).
    pub async fn ingest(&self, thread: ThreadId, body: &[u8]) -> Result<bool, DomainError> {
        let Some(export) = decode_token_export(body) else {
            return Ok(false);
        };
        let dedupe = format!(
            "otlp:{}:{:032x}",
            thread.value(),
            xxhash_rust::xxh3::xxh3_128(body)
        );
        let payload = AgentTokensReportedV1 {
            thread: thread_ref(thread),
            counts: export
                .counts
                .iter()
                .map(|c| TokenCount {
                    model: c.model.clone(),
                    kind: c.kind,
                    value: c.value.max(0) as u64,
                })
                .collect(),
            window_end: export.window_end.map(|t| t.to_text()),
        };
        let vocabulary = self.vocabulary.clone();
        let logged = self
            .db
            .transaction(move |tx| {
                let vocabulary = vocabulary.current();
                let mut anchors = activity_anchors_tx(tx, thread)?;
                // The turn it measured, not the one open as it arrives: an
                // export lands after its turn's Stop. One that doesn't say
                // when is the open turn's.
                if let Some(at) = export.window_end {
                    anchors.turn_id = turn_at_tx(tx, thread, at)?.map(|t| t.value());
                }
                let subject = anchors
                    .turn_id
                    .map(|t| turn_ref(oxplow_domain::AgentTurnId::new(t)))
                    .unwrap_or_else(|| thread_ref(thread));
                let env = EventCtx::system(&vocabulary, "otlp")
                    .typed::<AgentTokensReported>(&payload)
                    .with_anchors(anchors)
                    .with_subject([subject])
                    .with_dedupe_key(dedupe.clone());
                append_unique_tx(tx, &vocabulary, &env)
            })
            .await?;
        if logged {
            self.pump.wake();
        }
        Ok(logged)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hook_ingest::HookEnvelope;
    use crate::otlp_tokens::{encoded_claude_export, encoded_claude_export_at};
    use oxplow_domain::{HookKind, StoredEvent, Timestamp};

    fn hook(thread: ThreadId, kind: HookKind) -> HookEnvelope {
        HookEnvelope {
            kind,
            thread_id: Some(thread),
            stream_id: None,
            session_id: Some("s".into()),
            payload_json: "{}".into(),
            prompt: Some("go".into()),
            decision: None,
        }
    }

    async fn reported(svc: &crate::Services) -> Vec<StoredEvent> {
        svc.event_log_store
            .read_after(0, 1000)
            .await
            .unwrap()
            .into_iter()
            .filter(|e| e.envelope.event_type == "agent.tokens.reported")
            .collect()
    }

    async fn open_turn(svc: &crate::Services) -> i64 {
        svc.db
            .read(|c| {
                c.query_row(
                    "SELECT id FROM agent_turn WHERE ended_at IS NULL",
                    [],
                    |r| r.get(0),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap()
    }

    /// An export is one event: its counts, anchored to the open turn and
    /// the thread's effort.
    #[tokio::test]
    async fn an_export_logs_one_event_anchored_to_its_turn() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        svc.hook_ingest
            .ingest(hook(fx.thread, HookKind::UserPromptSubmit))
            .await
            .unwrap();
        let turn = open_turn(svc).await;
        let body = encoded_claude_export_at("claude-opus-4-8", 100, 20, Some(Timestamp::now()));
        assert!(svc.otlp_ingest.ingest(fx.thread, &body).await.unwrap());
        let events = reported(svc).await;
        assert_eq!(events.len(), 1);
        let env = &events[0].envelope;
        assert_eq!(env.anchors.turn_id, Some(turn));
        assert_eq!(env.anchors.effort_id, Some(fx.effort));
        assert_eq!(env.payload["counts"][0]["kind"], "input");
        assert_eq!(env.payload["counts"][0]["value"], 100);
    }

    /// An SDK retransmit of the same export logs nothing more; a body
    /// with no token counts logs nothing.
    #[tokio::test]
    async fn a_retransmit_logs_nothing() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let body = encoded_claude_export("claude-opus-4-8", 100, 20);
        assert!(svc.otlp_ingest.ingest(fx.thread, &body).await.unwrap());
        assert!(!svc.otlp_ingest.ingest(fx.thread, &body).await.unwrap());
        assert!(!svc
            .otlp_ingest
            .ingest(fx.thread, b"not otlp")
            .await
            .unwrap());
        assert_eq!(reported(svc).await.len(), 1);
    }

    /// An export arrives after the turn it measured has ended and the
    /// next begun: it is that turn's, by its own time window.
    #[tokio::test]
    async fn an_export_after_stop_lands_on_the_turn_it_measured() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        svc.hook_ingest
            .ingest(hook(fx.thread, HookKind::UserPromptSubmit))
            .await
            .unwrap();
        let measured_in = open_turn(svc).await;
        let at = Timestamp::now();
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        svc.hook_ingest
            .ingest(hook(fx.thread, HookKind::Stop))
            .await
            .unwrap();
        svc.hook_ingest
            .ingest(hook(fx.thread, HookKind::UserPromptSubmit))
            .await
            .unwrap();
        assert_ne!(open_turn(svc).await, measured_in);
        let body = encoded_claude_export_at("claude-opus-4-8", 100, 20, Some(at));
        svc.otlp_ingest.ingest(fx.thread, &body).await.unwrap();
        let events = reported(svc).await;
        assert_eq!(events[0].envelope.anchors.turn_id, Some(measured_in));
    }

    /// The facts come from the consumer, once, under a capture carrying
    /// the event's turn and effort — the ingest writes none.
    #[tokio::test]
    async fn otlp_facts_come_only_from_the_consumer() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        // The token facts are written only while a spec consumes them.
        svc.metrics.seed_catalog().await;
        svc.hook_ingest
            .ingest(hook(fx.thread, HookKind::UserPromptSubmit))
            .await
            .unwrap();
        let turn = open_turn(svc).await;
        let body = encoded_claude_export_at("claude-opus-4-8", 100, 20, Some(Timestamp::now()));
        svc.otlp_ingest.ingest(fx.thread, &body).await.unwrap();
        let tokens = svc
            .fact_store
            .get_measure("oxplow.tokens")
            .await
            .unwrap()
            .unwrap();
        assert!(svc
            .fact_store
            .facts_for_measure(tokens.id)
            .await
            .unwrap()
            .is_empty());
        svc.event_pump.run_once().await.unwrap();
        // A redelivery counts nothing twice.
        svc.event_log_store
            .set_checkpoint(crate::token_usage::OTLP_TOKENS.into(), 0)
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        let facts = svc.fact_store.facts_for_measure(tokens.id).await.unwrap();
        assert_eq!(facts.iter().map(|f| f.value).sum::<f64>(), 120.0);
        let (capture_turn, capture_effort): (Option<i64>, Option<i64>) = svc
            .db
            .read(|c| {
                c.query_row(
                    "SELECT turn_id, effort_id FROM metric_capture WHERE producer = 'otel-tokens'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .map_err(oxplow_db::map_sql_err)
            })
            .await
            .unwrap();
        assert_eq!(capture_turn, Some(turn));
        assert_eq!(capture_effort, Some(fx.effort.value()));
    }
}
