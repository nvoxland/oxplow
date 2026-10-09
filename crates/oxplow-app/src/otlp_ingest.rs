//! An agent's token telemetry in (P10.M2, `.context/agent-model.md` →
//! "Token usage"): the control plane's OTLP receiver (`POST /v1/metrics`,
//! `/v1/logs`) hands an export here. It is decoded and logged as one
//! `agent.tokens.reported` event in one transaction — anchored to the turn
//! the export measured (exports arrive after it; the one its window
//! overlaps most) and the effort open during that turn — and the `token_usage.otlp` consumer turns the event into
//! token facts. Nothing else writes them.

use std::sync::Arc;

use oxplow_db::agent_stores::{activity_anchors_tx, effort_during_turn_tx, turn_for_window_tx};
use oxplow_db::event_log_store::{append_unique_tx, EventCtx};
use oxplow_db::Database;
use oxplow_domain::agent::registry::HarnessRegistry;
use oxplow_domain::events::schema::{AgentTokensReported, AgentTokensReportedV1, TokenCount};
use oxplow_domain::refs::build::{thread_ref, turn_ref};
use oxplow_domain::vocabulary::VocabularyHandle;
use oxplow_domain::{AgentSessionId, DomainError, EffortId, ThreadId};

use crate::event_pump::EventPump;
use crate::otlp_tokens::decode_token_export;

/// Logs agents' token exports as events.
#[derive(Clone)]
pub struct OtlpIngestService {
    db: Database,
    vocabulary: VocabularyHandle,
    pump: Arc<EventPump>,
    /// What reads an export's token counts.
    harnesses: HarnessRegistry,
}

impl OtlpIngestService {
    pub fn new(
        db: Database,
        vocabulary: VocabularyHandle,
        pump: Arc<EventPump>,
        harnesses: HarnessRegistry,
    ) -> Self {
        Self {
            db,
            vocabulary,
            pump,
            harnesses,
        }
    }

    /// Log `thread`'s export `body` as `agent.tokens.reported`, from agent
    /// session `session` when the exporter named it (`X-Oxplow-Session`). Whether it
    /// logged one: a body with no token counts (most Codex log events)
    /// logs nothing, nor does a retransmit of an export already logged
    /// (keyed by the body's hash — each export carries its own window's
    /// timestamps, so two exports never hash alike).
    pub async fn ingest(
        &self,
        thread: ThreadId,
        session: Option<AgentSessionId>,
        body: &[u8],
    ) -> Result<bool, DomainError> {
        let Some(export) = decode_token_export(body, &self.harnesses).await else {
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
                    value: c.value as u64,
                })
                .collect(),
            window_end: export.window_end.map(|t| t.to_text()),
        };
        let vocabulary = self.vocabulary.clone();
        let logged = self
            .db
            .transaction(move |tx| {
                use rusqlite::OptionalExtension as _;
                // A thread that isn't there (deleted, or never) gets
                // nothing logged, as a hook for one doesn't (tsk925).
                let known = tx
                    .query_row(
                        "SELECT 1 FROM threads WHERE id = ?1",
                        [thread.value()],
                        |_| Ok(()),
                    )
                    .optional()
                    .map_err(oxplow_db::map_sql_err)?;
                if known.is_none() {
                    return Ok(false);
                }
                let vocabulary = vocabulary.current();
                let session =
                    oxplow_db::agent_session_store::resolve_tx(tx, thread, session, None)?
                        .map(|s| s.id);
                let mut anchors = activity_anchors_tx(tx, thread, session)?;
                // The turn it measured, not the one open as it arrives: an
                // export lands after its turn's Stop, and is stamped with
                // when it was collected, so its window can reach into the
                // next turn — it goes to the turn it overlaps most, and to
                // the effort open during that turn (tsk900). One that
                // doesn't say when is the open turn's.
                if let Some(end) = export.window_end {
                    let start = export.window_start.unwrap_or(end);
                    let turn = turn_for_window_tx(tx, thread, session, start, end)?;
                    anchors.turn_id = turn.map(|t| t.value());
                    anchors.effort_id = match turn {
                        Some(t) => effort_during_turn_tx(tx, thread, t)?.map(EffortId::new),
                        None => None,
                    };
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
    use crate::otlp_tokens::tests::{encoded_claude_export, encoded_claude_export_at};
    use oxplow_domain::{HookKind, StoredEvent, Timestamp};

    fn hook(thread: ThreadId, kind: HookKind) -> HookEnvelope {
        HookEnvelope {
            kind,
            thread_id: Some(thread),
            stream_id: None,
            agent_session_id: None,
            session_id: Some("s".into()),
            payload_json: "{}".into(),
            prompt: Some("go".into()),
            decision: None,
            tool: None,
            subagent: None,
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
        assert!(svc
            .otlp_ingest
            .ingest(fx.thread, None, &body)
            .await
            .unwrap());
        let events = reported(svc).await;
        assert_eq!(events.len(), 1);
        let env = &events[0].envelope;
        assert_eq!(env.anchors.turn_id, Some(turn));
        assert_eq!(env.anchors.effort_id, Some(fx.effort));
        assert_eq!(env.payload["counts"][0]["kind"], "input");
        assert_eq!(env.payload["counts"][0]["value"], 100);
    }

    /// Two sessions in a thread each have a turn running: an export from
    /// one (`X-Oxplow-Session`) is that session's turn's.
    #[tokio::test]
    async fn an_export_goes_to_its_own_sessions_turn() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let a = svc
            .agent_session_store
            .newest_for_thread(fx.thread)
            .await
            .unwrap()
            .unwrap()
            .id;
        let b = svc
            .db
            .transaction({
                let thread = fx.thread;
                move |tx| {
                    oxplow_db::agent_session_store::insert_tx(
                        tx,
                        &oxplow_domain::agent_session::NewAgentSession::terminal(thread, "claude"),
                        Timestamp::now(),
                    )
                }
            })
            .await
            .unwrap()
            .id;
        for (ses, sid) in [(a, "ha"), (b, "hb")] {
            svc.hook_ingest
                .ingest(HookEnvelope {
                    agent_session_id: Some(ses),
                    session_id: Some(sid.into()),
                    ..hook(fx.thread, HookKind::UserPromptSubmit)
                })
                .await
                .unwrap();
        }
        let turn_of = |ses: AgentSessionId| {
            svc.db.read(move |c| {
                c.query_row(
                    "SELECT id FROM agent_turn WHERE agent_session_id = ?1",
                    [ses.value()],
                    |r| r.get::<_, i64>(0),
                )
                .map_err(oxplow_db::map_sql_err)
            })
        };
        let (turn_a, turn_b) = (turn_of(a).await.unwrap(), turn_of(b).await.unwrap());
        let body = encoded_claude_export_at("claude-opus-4-8", 100, 20, Some(Timestamp::now()));
        assert!(svc
            .otlp_ingest
            .ingest(fx.thread, Some(a), &body)
            .await
            .unwrap());
        let body = encoded_claude_export_at("claude-opus-4-8", 5, 1, Some(Timestamp::now()));
        assert!(svc
            .otlp_ingest
            .ingest(fx.thread, Some(b), &body)
            .await
            .unwrap());
        let anchored: Vec<_> = reported(svc)
            .await
            .iter()
            .map(|e| {
                (
                    e.envelope.anchors.agent_session_id,
                    e.envelope.anchors.turn_id,
                )
            })
            .collect();
        assert_eq!(
            anchored,
            vec![(Some(a), Some(turn_a)), (Some(b), Some(turn_b))]
        );
    }

    /// An SDK retransmit of the same export logs nothing more; a body
    /// with no token counts logs nothing.
    #[tokio::test]
    async fn a_retransmit_logs_nothing() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let body = encoded_claude_export("claude-opus-4-8", 100, 20);
        assert!(svc
            .otlp_ingest
            .ingest(fx.thread, None, &body)
            .await
            .unwrap());
        assert!(!svc
            .otlp_ingest
            .ingest(fx.thread, None, &body)
            .await
            .unwrap());
        assert!(!svc
            .otlp_ingest
            .ingest(fx.thread, None, b"not otlp")
            .await
            .unwrap());
        assert_eq!(reported(svc).await.len(), 1);
    }

    /// tsk925: an export naming a thread that isn't there (deleted, or
    /// never) logs nothing, as a hook for one does.
    #[tokio::test]
    async fn an_export_for_an_unknown_thread_logs_nothing() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let body = encoded_claude_export("claude-opus-4-8", 100, 20);
        assert!(!svc
            .otlp_ingest
            .ingest(ThreadId::new(999), None, &body)
            .await
            .unwrap());
        assert!(reported(svc).await.is_empty());
    }

    /// tsk935: an export whose points say no time is the open turn's, and
    /// the open effort's.
    #[tokio::test]
    async fn an_export_with_no_point_time_is_the_open_turns() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        svc.hook_ingest
            .ingest(hook(fx.thread, HookKind::UserPromptSubmit))
            .await
            .unwrap();
        let open = open_turn(svc).await;
        let body = encoded_claude_export_at("claude-opus-4-8", 100, 20, None);
        assert!(svc
            .otlp_ingest
            .ingest(fx.thread, None, &body)
            .await
            .unwrap());
        let events = reported(svc).await;
        assert_eq!(events[0].envelope.anchors.turn_id, Some(open));
        assert_eq!(events[0].envelope.anchors.effort_id, Some(fx.effort));
    }

    /// tsk935: an export measured before the thread's first turn is no
    /// turn's — and no effort's.
    #[tokio::test]
    async fn an_export_before_the_first_turn_is_no_turns() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        let before = Timestamp::from_unix_ms(Timestamp::now().unix_ms() - 60_000);
        svc.hook_ingest
            .ingest(hook(fx.thread, HookKind::UserPromptSubmit))
            .await
            .unwrap();
        let body = encoded_claude_export_at("claude-opus-4-8", 100, 20, Some(before));
        assert!(svc
            .otlp_ingest
            .ingest(fx.thread, None, &body)
            .await
            .unwrap());
        let events = reported(svc).await;
        assert_eq!(events[0].envelope.anchors.turn_id, None);
        assert_eq!(events[0].envelope.anchors.effort_id, None);
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
        svc.otlp_ingest
            .ingest(fx.thread, None, &body)
            .await
            .unwrap();
        let events = reported(svc).await;
        assert_eq!(events[0].envelope.anchors.turn_id, Some(measured_in));
    }

    /// tsk900: a real Claude export is stamped with when it was collected,
    /// not when the response came back, so its window can span two turns.
    /// It goes to the turn its window overlaps most, and to the effort open
    /// during that turn — not the one open when it arrives.
    #[tokio::test]
    async fn an_export_spanning_two_turns_goes_to_the_one_it_overlaps_most() {
        let fx = crate::test_fixtures::services_with_effort().await;
        let svc = &fx.svc;
        for kind in [
            HookKind::UserPromptSubmit,
            HookKind::Stop,
            HookKind::UserPromptSubmit,
        ] {
            svc.hook_ingest.ingest(hook(fx.thread, kind)).await.unwrap();
        }
        let t0 = Timestamp::now().unix_ms() - 60_000;
        let at = move |s: i64| Timestamp::from_unix_ms(t0 + s * 1000);
        let (first_effort, thread) = (fx.effort.value(), fx.thread.value());
        let turns: Vec<i64> = svc
            .db
            .transaction(move |tx| {
                let ids: Vec<i64> = {
                    let mut st = tx
                        .prepare("SELECT id FROM agent_turn ORDER BY id")
                        .map_err(oxplow_db::map_sql_err)?;
                    let rows = st
                        .query_map([], |r| r.get(0))
                        .and_then(|r| r.collect::<rusqlite::Result<Vec<_>>>())
                        .map_err(oxplow_db::map_sql_err)?;
                    rows
                };
                // Turn A 0–10 s, turn B from 11 s; effort A ends at 10.5 s and
                // effort B opens at 11 s.
                let set = |sql: &str, p: Vec<String>| {
                    tx.execute(sql, rusqlite::params_from_iter(p))
                        .map(|_| ())
                        .map_err(oxplow_db::map_sql_err)
                };
                set(
                    "UPDATE agent_turn SET started_at = ?1, ended_at = ?2 WHERE id = ?3",
                    vec![at(0).to_string(), at(10).to_string(), ids[0].to_string()],
                )?;
                set(
                    "UPDATE agent_turn SET started_at = ?1, ended_at = NULL WHERE id = ?2",
                    vec![at(11).to_string(), ids[1].to_string()],
                )?;
                set(
                    "UPDATE effort SET started_at = ?1, ended_at = ?2 WHERE id = ?3",
                    vec![
                        at(0).to_string(),
                        Timestamp::from_unix_ms(t0 + 10_500).to_string(),
                        first_effort.to_string(),
                    ],
                )?;
                set(
                    "INSERT INTO effort (work_item, thread_id, started_at) VALUES (?1, ?2, ?3)",
                    vec![
                        "work_item:oxplow:tsk999".into(),
                        thread.to_string(),
                        at(11).to_string(),
                    ],
                )?;
                Ok(ids)
            })
            .await
            .unwrap();
        // Collected at 15 s over the window since the last export at 5 s:
        // 5 s of turn A, 4 s of turn B.
        let body = crate::otlp_tokens::tests::encoded_claude_export_over(
            "claude-opus-4-8",
            100,
            20,
            at(5),
            at(15),
        );
        svc.otlp_ingest
            .ingest(fx.thread, None, &body)
            .await
            .unwrap();
        let events = reported(svc).await;
        assert_eq!(events[0].envelope.anchors.turn_id, Some(turns[0]));
        assert_eq!(
            events[0].envelope.anchors.effort_id.map(|e| e.value()),
            Some(first_effort)
        );
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
        svc.otlp_ingest
            .ingest(fx.thread, None, &body)
            .await
            .unwrap();
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
