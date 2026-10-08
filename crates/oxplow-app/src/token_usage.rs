//! Agent token-usage capture (tsk104).
//!
//! **Two sources, split by tsk22.** The durable `oxplow.tokens` **facts** now
//! come from **OpenTelemetry**: the control-plane `POST /v1/metrics` receiver
//! logs each export as `agent.tokens.reported` ([`crate::otlp_ingest`], decode
//! in [`crate::otlp_tokens`]) and the `token_usage.otlp` consumer counts it
//! ([`TokenUsageService::record_reported`]). OTEL is accurate (the agent's own billed counts),
//! multi-agent, and format-stable; the old transcript parse overcounted ~2–3×
//! because Claude repeats a message's cumulative `usage` on every content-block
//! JSONL line and the parse summed every line (counting each `message.id`
//! once, in the Claude harness, removed that).
//!
//! The **transcript path** ([`TokenUsageService::on_stop`], run by the
//! `token_usage.turns` reactor on `agent.turn.ended`) survives for what OTEL
//! lacks: the per-turn `agent_token_usage` rows (with the human prompt text)
//! and the `oxplow.turn` facts. The event carries the Stop's
//! `transcript_path` (the agent session JSONL); for Claude each
//! `type=="assistant"` line carries a `message.usage` block + `message.model`.
//! We read the NEW records since the last turn (offset-tracked via a
//! persisted per-session cursor that commits with the rows, so we never re-sum
//! the whole file or double-count across restarts or redeliveries) and persist
//! one row per turn attributed to the effort the turn ran in. An ACP turn's
//! counts ride the event instead ([`TokenUsageService::record_turn`]).
//! Provenance is `observed`.
//!
//! Each harness reads its own transcript (`AgentHarness::turns`): Claude's
//! is read; Codex's and opencode's formats aren't yet. We track
//! token counts only; oxplow deliberately does not derive a USD price (rates
//! move and a stale price table is worse than none). The per-turn `model` is
//! stored so usage can be sliced by model.
//!
//! Mirrors the collection side-band (`collection.rs`): find open effort →
//! record → emit, best-effort. See `.context/agent-model.md`,
//! `.context/metrics.md`, + `.context/data-model.md`.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxplow_db::EffortStore;
use oxplow_db::{
    NewAgentTokenUsage, NewFact, NewMetricCapture, SqliteAgentSessionStore, SqliteEffortStore,
    SqliteFactStore, SqliteThreadStore, SqliteTokenUsageStore,
};
use oxplow_domain::agent::observe::{Turn, UsageDelta};
use oxplow_domain::agent::registry::HarnessRegistry;
use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{DomainError, StreamId, ThreadId};

/// Pull `transcript_path` out of a raw hook payload body, expanding a
/// leading `~/`.
fn extract_transcript_path(payload_json: &str) -> Option<PathBuf> {
    let v: serde_json::Value = serde_json::from_str(payload_json).ok()?;
    let raw = v.get("transcript_path")?.as_str()?;
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return Some(Path::new(&home).join(rest));
        }
    }
    Some(PathBuf::from(raw))
}

/// Read the transcript tail starting at `offset`, returning only the
/// COMPLETE lines (everything up to and including the last newline) plus
/// the new offset (just past that last newline). Bytes after the last
/// newline are an in-flight partial line and are left for the next read.
///
/// Returns `None` when there is nothing new to read (offset at EOF, no
/// complete line yet, or the file can't be opened). If the file shrank
/// below `offset` (rotated / truncated) we restart from 0.
fn read_complete_tail(path: &Path, offset: u64) -> Option<(String, u64)> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = if offset > len { 0 } else { offset };
    if start >= len {
        return None;
    }
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::with_capacity((len - start) as usize);
    file.read_to_end(&mut buf).ok()?;
    let last_nl = buf.iter().rposition(|&b| b == b'\n')?;
    let complete = String::from_utf8_lossy(&buf[..=last_nl]).into_owned();
    Some((complete, start + last_nl as u64 + 1))
}

/// Byte offset just past the last complete (newline-terminated) line in
/// `path` — the cursor position that skips all currently-written history
/// while leaving any in-flight partial tail for the next read. Returns 0
/// when the file has no complete line yet or can't be opened. Used to SEED
/// the cursor on the first capture for a session so the prior transcript is
/// never ingested as one lump (tsk142).
fn complete_offset(path: &Path) -> u64 {
    let Ok(mut file) = std::fs::File::open(path) else {
        return 0;
    };
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return 0;
    }
    match buf.iter().rposition(|&b| b == b'\n') {
        Some(pos) => pos as u64 + 1,
        None => 0,
    }
}

/// Per-(stop, model) turn count, accumulated across the turns in one Stop so
/// the substrate gets one `oxplow.turn` fact per model rather than one per turn.
/// Token totals are no longer projected here — OTEL owns the `oxplow.tokens`
/// facts (epic tsk22); this path keeps only the turn count (which the parser
/// gets right per genuine user prompt) plus the per-turn `agent_token_usage`
/// rows (with prompt text OTEL lacks).
#[derive(Default)]
struct TokenAgg {
    turns: i64,
}

/// Captures per-turn token usage from the agent transcript on Stop.
/// What [`TokenUsageService::insert_turns`] wrote, for `announce`.
/// The turn a token record is for, as the `token_usage.turns` reactor
/// knows it (P3.7). Empty for the direct callers (tests, older paths).
#[derive(Debug, Clone, Default)]
pub struct TurnRecord {
    /// `agent_turn.id`.
    pub turn_id: Option<i64>,
    /// The effort open when the turn ran (the event's anchor).
    pub effort: Option<oxplow_domain::EffortId>,
    /// The agent session it ran in (the event's anchor): its harness says
    /// how to read the transcript.
    pub agent_session: Option<oxplow_domain::AgentSessionId>,
    /// The `agent.turn.ended` event id. It keys the `oxplow.turn` facts
    /// capture, and — only for counts the turn reported itself (one row per
    /// event) — the row, so a redelivery records them once. Transcript rows
    /// are not keyed by it: a chunk can hold several turns (tsk498), and
    /// their idempotency is the cursor committing with them.
    pub cause: Option<String>,
}

/// Where a turn's counts came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Counted {
    /// Parsed from the session transcript's new tail (Claude).
    Transcript,
    /// Reported by the harness with the turn (ACP).
    Reported,
}

struct RecordedTurns {
    last_id: Option<i64>,
    effort_val: Option<i64>,
    by_model: std::collections::HashMap<String, TokenAgg>,
}

#[derive(Clone)]
pub struct TokenUsageService {
    usage: Arc<SqliteTokenUsageStore>,
    efforts: Arc<SqliteEffortStore>,
    threads: Arc<SqliteThreadStore>,
    sessions: Arc<SqliteAgentSessionStore>,
    /// Durable fact layer (epic tsk12): per-kind token totals land as facts
    /// on the `oxplow.tokens` measure.
    facts: Arc<SqliteFactStore>,
    /// Who reads a transcript: the turn's harness (`AgentHarness::turns`).
    harnesses: HarnessRegistry,
}

impl TokenUsageService {
    pub fn new(
        usage: Arc<SqliteTokenUsageStore>,
        efforts: Arc<SqliteEffortStore>,
        threads: Arc<SqliteThreadStore>,
        sessions: Arc<SqliteAgentSessionStore>,
        facts: Arc<SqliteFactStore>,
        harnesses: HarnessRegistry,
    ) -> Self {
        Self {
            usage,
            efforts,
            threads,
            sessions,
            facts,
            harnesses,
        }
    }

    /// The harness that ran a turn: its agent session's; a turn no session
    /// claims reads as the default harness's.
    async fn harness_of(&self, rec: &TurnRecord) -> Result<String, DomainError> {
        use oxplow_domain::stores::AgentSessionStore as _;
        let session = match rec.agent_session {
            Some(id) => self.sessions.get(&id).await?.map(|s| s.harness),
            None => None,
        };
        Ok(session
            .or_else(|| self.harnesses.default().ok().map(|h| h.id().to_string()))
            .unwrap_or_default())
    }

    /// On Stop: parse the transcript tail since the last cursor, sum usage,
    /// and persist one row attributed to the open effort + thread. Returns
    /// `Ok(Some(id))` when a row was written, `Ok(None)` when there was
    /// nothing to record (no transcript_path, no thread, no new usage, or a
    /// non-Claude agent). Best-effort — the caller treats errors as
    /// non-fatal so a parse hiccup never blocks the hook.
    /// The rows carry the turn (`rec`) the `token_usage.turns` reactor saw
    /// and the effort it ran in.
    pub async fn on_stop(
        &self,
        thread: &ThreadId,
        session_id: Option<&str>,
        payload_json: &str,
        rec: &TurnRecord,
    ) -> Result<Option<i64>, DomainError> {
        let Some(transcript_path) = extract_transcript_path(payload_json) else {
            return Ok(None);
        };
        let Some(thread_row) = self.threads.get(thread).await? else {
            return Ok(None);
        };
        let kind = self.harness_of(rec).await?;
        let stream_id = thread_row.stream_id.to_string();

        // Cursor key: the session id (1:1 with the transcript for Claude),
        // falling back to the path when the hook omitted a session id.
        let session_key = session_id
            .map(str::to_string)
            .unwrap_or_else(|| transcript_path.to_string_lossy().into_owned());

        // First capture for this session (fresh daemon, or first Stop after
        // attaching to an already-long transcript): there is no stored cursor.
        // SEED it to the current end-of-history WITHOUT ingesting the prior
        // transcript — otherwise the whole file lands as one giant turns:1
        // lump. We only attribute tokens spent while oxplow was watching
        // (tsk142). A real `Some(0)` cursor (a session we genuinely started
        // at byte 0) still takes the normal ingest path.
        let offset = match self.usage.cursor(&session_key).await? {
            Some(offset) => offset,
            None => {
                let seed = complete_offset(&transcript_path);
                self.usage.set_cursor(&session_key, seed).await?;
                return Ok(None);
            }
        };
        let Some((tail, new_offset)) = read_complete_tail(&transcript_path, offset) else {
            return Ok(None);
        };

        // The turn's harness reads its own transcript; one no longer
        // registered reads as none.
        let turns = self
            .harnesses
            .get(&kind)
            .map(|h| h.turns(&tail))
            .unwrap_or_default();
        if turns.is_empty() {
            // Nothing to record from this chunk (no usage / no prompt / a
            // harness that reads no transcript), but the bytes are consumed —
            // advance so we don't re-scan them every Stop.
            self.usage.set_cursor(&session_key, new_offset).await?;
            return Ok(None);
        }
        // The rows and the cursor past their bytes commit together, so a
        // redelivered turn never reads (and records) the same bytes again.
        let recorded = self
            .insert_turns(
                thread,
                &stream_id,
                kind,
                &session_key,
                turns,
                rec,
                Counted::Transcript,
                Some((session_key.clone(), new_offset)),
            )
            .await?;
        self.announce(thread, &stream_id, &recorded, rec).await;
        Ok(recorded.last_id)
    }

    /// Record one ACP turn's token counts (from the prompt response), the
    /// counterpart of [`Self::on_stop`]'s transcript parse. `Ok(None)` when
    /// the thread is gone or the counts are empty.
    /// Keyed by its `agent.turn.ended` (`rec.cause`), so a redelivery
    /// counts it once.
    pub async fn record_turn(
        &self,
        thread: &ThreadId,
        session_id: &str,
        turn: Turn,
        rec: &TurnRecord,
    ) -> Result<Option<i64>, DomainError> {
        if !turn.is_recordable() {
            return Ok(None);
        }
        let Some(thread_row) = self.threads.get(thread).await? else {
            return Ok(None);
        };
        let stream_id = thread_row.stream_id.to_string();
        let recorded = self
            .insert_turns(
                thread,
                &stream_id,
                self.harness_of(rec).await?,
                session_id,
                vec![turn],
                rec,
                Counted::Reported,
                None,
            )
            .await?;
        self.announce(thread, &stream_id, &recorded, rec).await;
        Ok(recorded.last_id)
    }

    /// One row per turn, and `cursor` advanced, in one transaction.
    /// [`Self::announce`] then emits and projects them.
    #[allow(clippy::too_many_arguments)]
    async fn insert_turns(
        &self,
        thread: &ThreadId,
        stream_id: &str,
        kind: String,
        session_key: &str,
        turns: Vec<Turn>,
        rec: &TurnRecord,
        counted: Counted,
        cursor: Option<(String, u64)>,
    ) -> Result<RecordedTurns, DomainError> {
        // The effort the turn ran in (its event's anchor), else the
        // thread's open one. An effort opened later adopts the row.
        let open_effort = match rec.effort {
            Some(id) => self.efforts.get_effort(&id).await?,
            None => self.efforts.find_open_for_thread(thread).await?,
        };
        let effort_id = open_effort.as_ref().map(|e| e.id.to_string());
        // The i64 form stamps the fact-capture so `captures_for_effort` (the T-D
        // fact-attribution read) attributes the token facts (tsk37).
        let effort_val = open_effort.as_ref().map(|e| e.id.value());

        // One row per turn — each carrying its opening prompt, model, and the
        // usage of the assistant messages that answered it (tsk143). While we
        // record, accumulate per-model totals for the metric projection.
        let mut rows = Vec::new();
        let mut by_model: std::collections::HashMap<String, TokenAgg> =
            std::collections::HashMap::new();
        for turn in turns {
            let model_key = turn.usage.model.clone().unwrap_or_else(|| "unknown".into());
            let (input, output, cc, cr) = (
                turn.usage.input_tokens,
                turn.usage.output_tokens,
                turn.usage.cache_creation_input_tokens,
                turn.usage.cache_read_input_tokens,
            );
            rows.push(NewAgentTokenUsage {
                stream_id: stream_id.to_string(),
                thread_id: thread.to_string(),
                effort_id: effort_id.clone(),
                session_id: session_key.to_string(),
                agent_kind: kind.clone(),
                model: turn.usage.model,
                prompt: turn.prompt,
                input_tokens: input,
                output_tokens: output,
                cache_creation_input_tokens: cc,
                cache_read_input_tokens: cr,
                message_count: turn.usage.message_count,
                turn_id: rec.turn_id,
                cause: match counted {
                    Counted::Reported => rec.cause.clone(),
                    Counted::Transcript => None,
                },
            });
            by_model.entry(model_key).or_default().turns += 1;
        }
        let ids = self.usage.record_batch(rows, cursor).await?;
        if ids.is_empty() {
            // Every row was already recorded (a redelivered turn): nothing
            // new to announce or project.
            by_model.clear();
        }
        let last_id = ids.last().copied();
        Ok(RecordedTurns {
            last_id,
            effort_val,
            by_model,
        })
    }

    /// Tell the UI and project the recorded turns into the metric substrate
    /// (best-effort).
    async fn announce(
        &self,
        thread: &ThreadId,
        stream_id: &str,
        recorded: &RecordedTurns,
        rec: &TurnRecord,
    ) {
        self.project_token_metrics(
            thread,
            stream_id,
            &recorded.by_model,
            recorded.effort_val,
            rec,
        )
        .await;
    }

    /// Project per-model token totals into the metric substrate. Best-effort: a
    /// metric write error is logged, never fails the Stop hook. No branch
    /// dimension (operational metric, not a code fact).
    async fn project_token_metrics(
        &self,
        thread: &ThreadId,
        stream_id: &str,
        by_model: &std::collections::HashMap<String, TokenAgg>,
        effort_val: Option<i64>,
        rec: &TurnRecord,
    ) {
        if by_model.is_empty() {
            return;
        }
        let Some(stream_val) = StreamId::try_from_str(stream_id).map(|s| s.value()) else {
            return;
        };
        if let Err(e) = self
            .record_token_metrics(thread, stream_val, by_model, effort_val, rec)
            .await
        {
            tracing::warn!(error = %e, "failed to project token usage into metric substrate");
        }
    }

    /// Record the turn facts (the change loop announces them, P7.B1).
    async fn record_token_metrics(
        &self,
        thread: &ThreadId,
        stream_val: i64,
        by_model: &std::collections::HashMap<String, TokenAgg>,
        effort_val: Option<i64>,
        rec: &TurnRecord,
    ) -> Result<(), DomainError> {
        // Turn facts only (epic tsk22): the `oxplow.tokens` facts now come from
        // the OTEL producer (`record_reported`) — accurate + multi-agent —
        // so the transcript path projects just the `oxplow.turn` count (one
        // fact per model; additive event measure, model a conformed dimension).
        // The per-turn `agent_token_usage` rows (with prompt text) are recorded
        // above; those are what this path still uniquely provides.
        // Stop-collecting gate (tsk31): skip the turn facts when `agent.turns` is
        // disabled (nothing consumes `oxplow.turn`).
        if !self
            .facts
            .measure_has_active_spec("oxplow.turn")
            .await
            .unwrap_or(true)
        {
            return Ok(());
        }
        let turn_measure = self.facts.get_measure("oxplow.turn").await?;
        if let Some(tm) = turn_measure {
            let mut facts = Vec::new();
            for (model, agg) in by_model {
                if agg.turns > 0 {
                    facts.push(NewFact {
                        subject_kind: Some("model".into()),
                        subject_ref: Some(format!("model:{model}")),
                        // json! (not format!) — the model id comes verbatim from
                        // external session JSONL; a quote/backslash in it must
                        // not poison the dims JSON (tsk46).
                        dims_json: Some(serde_json::json!({ "oxplow.model": model }).to_string()),
                        ..NewFact::new(tm.id, agg.turns as f64)
                    });
                }
            }
            if !facts.is_empty() {
                let mut capture = NewMetricCapture::done(stream_val, "token-parse", "token-parse");
                capture.thread_id = Some(thread.value());
                capture.trigger = Some("continuous".into());
                capture.effort_id = effort_val;
                capture.turn_id = rec.turn_id;
                capture.idempotency_key = rec.cause.as_ref().map(|c| format!("turn-tokens:{c}"));
                self.facts.record_facts(capture, facts).await?;
            }
        }
        Ok(())
    }

    /// Count an agent's reported tokens (`agent.tokens.reported`, logged
    /// by [`crate::otlp_ingest`] from an OTLP export — the OpenTelemetry
    /// successor to the transcript-parse token facts, epic tsk22): per
    /// model and kind, input/output on `oxplow.tokens`, cache kinds on
    /// `oxplow.cache_tokens`, plus one per-model `oxplow.cache_usage`
    /// hit-ratio fact (tsk73) — all under one capture carrying the event's
    /// thread, stream, effort and turn, keyed by the event so a redelivery
    /// counts nothing twice. How many facts it wrote.
    pub async fn record_reported(
        &self,
        event: &oxplow_domain::StoredEvent,
    ) -> Result<usize, DomainError> {
        use oxplow_domain::events::schema::{AgentTokensReportedV1, TokenKind};
        let env = &event.envelope;
        let (Some(thread), Some(stream)) = (env.anchors.thread_id, env.anchors.stream_id) else {
            return Ok(0);
        };
        let reported: AgentTokensReportedV1 = serde_json::from_value(env.payload.clone())
            .map_err(|e| DomainError::Invalid(format!("agent.tokens.reported: {e}")))?;
        // Stop-collecting gates (tsk31), one per measure this export can feed:
        // input/output → `oxplow.tokens` (the agent.tokens.* specs), cache
        // kinds → `oxplow.cache_tokens`, and the per-model hit-ratio fact →
        // `oxplow.cache_usage` (tsk73). Cache rides SEPARATE measures because
        // `agent.tokens.total` is an unfiltered sum over `oxplow.tokens` —
        // cache facts there would silently change its meaning.
        let tokens_measure = self.active_measure("oxplow.tokens").await?;
        let cache_measure = self.active_measure("oxplow.cache_tokens").await?;
        let usage_measure = self.active_measure("oxplow.cache_usage").await?;
        if tokens_measure.is_none() && cache_measure.is_none() && usage_measure.is_none() {
            return Ok(0);
        }
        // json! (not format!) — the model id is verbatim from the export; a
        // quote/backslash must not poison the dims JSON.
        let kind_dims = |model: &str, kind: &str| {
            serde_json::json!({
                "oxplow.model": model,
                "oxplow.token_kind": kind,
            })
            .to_string()
        };
        let mut facts: Vec<NewFact> = Vec::new();
        for c in &reported.counts {
            let measure = if c.kind.is_cache() {
                &cache_measure
            } else {
                &tokens_measure
            };
            let Some(m) = measure else { continue };
            facts.push(NewFact {
                subject_kind: Some("model".into()),
                subject_ref: Some(format!("model:{}", c.model)),
                dims_json: Some(kind_dims(&c.model, c.kind.as_str())),
                ..NewFact::new(m.id, c.value as f64)
            });
        }
        // Per-model prompt-cache hit ratio (tsk73): num = cache_read, den =
        // input + cache_read + cache_creation (prompt-side; output can't be
        // cached). Emitted only when the export carried cache telemetry at
        // all — an agent that doesn't report cache kinds must read as "no
        // data", not a string of 0% points dragging the cumulative Σn/Σd.
        if let Some(um) = &usage_measure {
            let mut by_model: std::collections::BTreeMap<&str, (f64, f64, f64)> =
                std::collections::BTreeMap::new();
            for c in &reported.counts {
                let e = by_model.entry(c.model.as_str()).or_default();
                match c.kind {
                    TokenKind::Input => e.0 += c.value as f64,
                    TokenKind::CacheRead => e.1 += c.value as f64,
                    TokenKind::CacheCreation => e.2 += c.value as f64,
                    TokenKind::Output => {}
                }
            }
            for (model, (input, cache_read, cache_creation)) in by_model {
                let den = input + cache_read + cache_creation;
                if den <= 0.0 || (cache_read <= 0.0 && cache_creation <= 0.0) {
                    continue;
                }
                facts.push(NewFact {
                    subject_kind: Some("model".into()),
                    subject_ref: Some(format!("model:{model}")),
                    numerator: Some(cache_read),
                    denominator: Some(den),
                    dims_json: Some(serde_json::json!({ "oxplow.model": model }).to_string()),
                    ..NewFact::new(um.id, cache_read / den * 100.0)
                });
            }
        }
        if facts.is_empty() {
            return Ok(0);
        }
        let count = facts.len();
        let mut capture = NewMetricCapture::done(stream.value(), "otel-tokens", "otel");
        capture.thread_id = Some(thread.value());
        capture.trigger = Some("continuous".into());
        capture.effort_id = env.anchors.effort_id.map(|e| e.value());
        capture.turn_id = env.anchors.turn_id;
        capture.idempotency_key = Some(format!("otel-tokens:{}", env.id.as_str()));
        self.facts.record_facts(capture, facts).await?;
        Ok(count)
    }

    /// `key`'s measure, when a spec still consumes it (tsk31).
    async fn active_measure(&self, key: &str) -> Result<Option<oxplow_db::Measure>, DomainError> {
        if !self
            .facts
            .measure_has_active_spec(key)
            .await
            .unwrap_or(true)
        {
            return Ok(None);
        }
        self.facts.get_measure(key).await
    }
}

/// The OTLP consumer's name (its checkpoint key).
pub const OTLP_TOKENS: &str = "token_usage.otlp";

/// Counts an agent's reported tokens (`agent.tokens.reported`) into facts
/// (P10.M2): the only writer of the OTLP token facts.
pub struct OtlpTokensConsumer {
    pub tokens: TokenUsageService,
}

#[async_trait::async_trait]
impl crate::event_pump::AsyncEventConsumer for OtlpTokensConsumer {
    fn name(&self) -> &'static str {
        OTLP_TOKENS
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == "agent.tokens.reported"
    }

    async fn handle(&self, event: &oxplow_domain::StoredEvent) -> Result<(), DomainError> {
        self.tokens.record_reported(event).await.map(|_| ())
    }
}

/// The reactor's consumer name (its checkpoint key).
pub const TURN_TOKENS: &str = "token_usage.turns";

/// Counts a turn's tokens when it ends (P3.7): from the counts the turn
/// reported itself (ACP, `usage` on `agent.turn.ended@2`) or from the
/// session transcript's new tail (Claude, `transcript_path`). An async
/// pump consumer — no transcript parsing in the Stop hook — anchored to
/// the turn and the effort it ran in.
pub struct TurnTokensConsumer {
    pub tokens: TokenUsageService,
    pub turns: oxplow_db::SqliteAgentTurnStore,
}

#[async_trait::async_trait]
impl crate::event_pump::AsyncEventConsumer for TurnTokensConsumer {
    fn name(&self) -> &'static str {
        TURN_TOKENS
    }

    fn handles(&self, event_type: &str) -> bool {
        event_type == "agent.turn.ended"
    }

    async fn handle(&self, event: &oxplow_domain::StoredEvent) -> Result<(), DomainError> {
        use oxplow_domain::stores::AgentTurnStore as _;
        let env = &event.envelope;
        let (Some(thread), Some(turn_id)) = (env.anchors.thread_id, env.anchors.turn_id) else {
            return Ok(());
        };
        let payload: oxplow_domain::events::schema::AgentTurnEndedV2 =
            serde_json::from_value(env.payload.clone())
                .map_err(|e| DomainError::Invalid(format!("agent.turn.ended: {e}")))?;
        let turn = self
            .turns
            .get(&oxplow_domain::AgentTurnId::new(turn_id))
            .await?;
        let session = turn.as_ref().and_then(|t| t.session_id.clone());
        let rec = TurnRecord {
            turn_id: Some(turn_id),
            effort: env.anchors.effort_id,
            agent_session: env.anchors.agent_session_id,
            cause: Some(env.id.as_str().to_string()),
        };
        if let Some(u) = payload.usage {
            // Rows key on the session; a turn the harness reported counts for
            // always has one (ACP opens turns with a session id).
            let Some(session) = session.as_deref().filter(|s| !s.is_empty()) else {
                tracing::warn!(
                    turn = turn_id,
                    "turn reported usage but has no session; not counted"
                );
                return Ok(());
            };
            let reported = Turn {
                prompt: turn.map(|t| t.prompt).filter(|p| !p.is_empty()),
                usage: UsageDelta {
                    input_tokens: u.input as i64,
                    output_tokens: u.output as i64,
                    cache_creation_input_tokens: u.cache_write as i64,
                    cache_read_input_tokens: u.cache_read as i64,
                    message_count: 1,
                    model: u.model,
                },
            };
            self.tokens
                .record_turn(&thread, session, reported, &rec)
                .await?;
        } else if let Some(path) = payload.transcript_path {
            let body = serde_json::json!({ "transcript_path": path }).to_string();
            self.tokens
                .on_stop(&thread, session.as_deref(), &body, &rec)
                .await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxplow_tasks::work_item_ref;
    use std::io::Write;

    const ASSISTANT_LINE: &str = r#"{"type":"assistant","message":{"model":"claude-opus-4-8","usage":{"input_tokens":100,"output_tokens":20,"cache_creation_input_tokens":50,"cache_read_input_tokens":200}}}"#;

    #[test]
    fn extract_transcript_path_reads_field() {
        let p = extract_transcript_path(r#"{"transcript_path":"/tmp/x.jsonl","session_id":"s"}"#);
        assert_eq!(p, Some(PathBuf::from("/tmp/x.jsonl")));
        assert!(extract_transcript_path("{}").is_none());
        assert!(extract_transcript_path("not json").is_none());
    }

    #[test]
    fn read_complete_tail_only_returns_whole_lines() {
        let dir = std::env::temp_dir().join(format!("oxplow-tu-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        // Two complete lines + a partial third (no trailing newline).
        std::fs::write(&path, "line1\nline2\npartial").unwrap();
        let (tail, off) = read_complete_tail(&path, 0).unwrap();
        assert_eq!(tail, "line1\nline2\n");
        assert_eq!(off, "line1\nline2\n".len() as u64);
        // From the new offset, only the partial remains — no complete line.
        assert!(read_complete_tail(&path, off).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Build a real in-memory `Services` over a fresh git repo, seed a
    /// primary stream + a Claude thread, and return the service + thread id.
    /// Token usage is attributed to the thread (no open effort created — the
    /// effort-attribution path is covered by the store's `totals_for_effort`
    /// tests).
    async fn service_fixture() -> (std::sync::Arc<crate::Services>, tempfile::TempDir, ThreadId) {
        let dir = tempfile::tempdir().unwrap();
        crate::test_fixtures::init_git_repo(dir.path());
        let svc = std::sync::Arc::new(crate::Services::in_memory(dir.path()).unwrap());
        // Seed the catalog as boot does — the token/turn producers gate collection
        // on `measure_has_active_spec` (tsk31), so the specs must exist.
        svc.metrics.seed_catalog().await;
        let stream = svc.streams.ensure_primary().await.unwrap();
        let thread = crate::test_fixtures::new_thread(&svc, stream.id, "T").await;
        (svc, dir, thread.id)
    }

    fn stop(thread: ThreadId, body: serde_json::Value) -> crate::HookEnvelope {
        crate::HookEnvelope {
            kind: oxplow_domain::HookKind::Stop,
            thread_id: Some(thread),
            stream_id: None,
            agent_session_id: None,
            session_id: Some("sess-r".into()),
            payload_json: body.to_string(),
            prompt: None,
            decision: None,
        }
    }

    async fn prompt(svc: &crate::Services, thread: ThreadId) -> oxplow_domain::AgentTurnId {
        svc.hook_ingest
            .ingest(crate::HookEnvelope {
                kind: oxplow_domain::HookKind::UserPromptSubmit,
                thread_id: Some(thread),
                stream_id: None,
                agent_session_id: None,
                session_id: Some("sess-r".into()),
                payload_json: "{}".into(),
                prompt: Some("count me".into()),
                decision: None,
            })
            .await
            .unwrap();
        use oxplow_domain::stores::AgentTurnStore as _;
        svc.agent_turn_store.list_open(&thread).await.unwrap()[0].id
    }

    /// P3.7 (tsk477): a turn's tokens are counted by the `token_usage.turns`
    /// reactor from its `agent.turn.ended` — the transcript tail for Claude
    /// — anchored to the turn and its effort; a redelivery reads nothing
    /// twice because the cursor moved with the rows.
    #[tokio::test]
    async fn a_turns_transcript_is_counted_once_by_the_reactor() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        svc.metrics.seed_catalog().await;
        let tdir = tempfile::tempdir().unwrap();
        let path = tdir.path().join("session.jsonl");
        std::fs::write(&path, "").unwrap();
        svc.token_usage_store.set_cursor("sess-r", 0).await.unwrap();
        let turn = prompt(svc, f.thread).await;
        std::fs::write(&path, format!("{ASSISTANT_LINE}\n")).unwrap();
        svc.hook_ingest
            .ingest(stop(
                f.thread,
                serde_json::json!({"transcript_path": path.to_string_lossy()}),
            ))
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        svc.event_log_store
            .set_checkpoint(TURN_TOKENS.into(), 0)
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        let rows = svc
            .token_usage_store
            .list_for_effort(&f.effort.to_string())
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].input_tokens, 100);
        let turn_ids: Vec<Option<i64>> = {
            let sl = crate::sql_gateway::SqlGateway::new(svc.db.clone());
            let r = sl
                .query_sql("SELECT turn_id FROM v_token_usage", vec![], None)
                .await
                .unwrap()
                .rows;
            serde_json::from_value(serde_json::to_value(r).unwrap())
                .map(|v: Vec<Vec<Option<i64>>>| v.into_iter().map(|r| r[0]).collect())
                .unwrap()
        };
        assert_eq!(turn_ids, vec![Some(turn.value())]);
        // tsk483: the turn facts' capture carries its turn too.
        let capture_turns: Vec<Option<i64>> = {
            let sl = crate::sql_gateway::SqlGateway::new(svc.db.clone());
            let r = sl
                .query_sql(
                    "SELECT turn_id FROM v_capture WHERE producer = 'token-parse'",
                    vec![],
                    None,
                )
                .await
                .unwrap()
                .rows;
            serde_json::from_value(serde_json::to_value(r).unwrap())
                .map(|v: Vec<Vec<Option<i64>>>| v.into_iter().map(|r| r[0]).collect())
                .unwrap()
        };
        assert_eq!(capture_turns, vec![Some(turn.value())]);
    }

    /// An ACP turn reports its own counts; they ride its `agent.turn.ended`
    /// and are counted once, keyed by that event.
    #[tokio::test]
    async fn a_turns_own_usage_is_counted_once() {
        let f = crate::test_fixtures::services_with_effort().await;
        let svc = &f.svc;
        prompt(svc, f.thread).await;
        svc.hook_ingest
            .ingest(stop(
                f.thread,
                serde_json::json!({"oxplow_turn_usage": {"input": 7, "output": 3, "cache_write": 0, "cache_read": 1}}),
            ))
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        svc.event_log_store
            .set_checkpoint(TURN_TOKENS.into(), 0)
            .await
            .unwrap();
        svc.event_pump.run_once().await.unwrap();
        let rows = svc
            .token_usage_store
            .list_for_effort(&f.effort.to_string())
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            (rows[0].input_tokens, rows[0].prompt.as_deref()),
            (7, Some("count me"))
        );
    }

    #[tokio::test]
    async fn on_stop_records_only_the_new_delta_incrementally() {
        let (svc, _dir, thread) = service_fixture().await;
        let tdir = tempfile::tempdir().unwrap();
        let path = tdir.path().join("session.jsonl");
        let thread_key = thread.to_string();
        let payload = format!(
            "{{\"transcript_path\":{:?},\"session_id\":\"sess-1\"}}",
            path.to_string_lossy()
        );

        // Bootstrap Stop: the session already has one assistant message when
        // oxplow first sees it. The first capture seeds the cursor to the end
        // WITHOUT recording (history isn't attributed; tsk142).
        std::fs::write(&path, format!("{ASSISTANT_LINE}\n")).unwrap();
        let id0 = svc
            .token_usage
            .on_stop(&thread, Some("sess-1"), &payload, &TurnRecord::default())
            .await
            .unwrap();
        assert!(id0.is_none(), "bootstrap Stop seeds, records nothing");
        assert_eq!(
            svc.token_usage_store
                .totals_for_thread(&thread_key)
                .await
                .unwrap()
                .turns,
            0
        );

        // First watched turn: append one assistant message. The new row must
        // reflect ONLY the appended delta, not the whole file.
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            f.write_all(format!("{ASSISTANT_LINE}\n").as_bytes())
                .unwrap();
        }
        let id1 = svc
            .token_usage
            .on_stop(&thread, Some("sess-1"), &payload, &TurnRecord::default())
            .await
            .unwrap();
        assert!(id1.is_some());
        let t = svc
            .token_usage_store
            .totals_for_thread(&thread_key)
            .await
            .unwrap();
        assert_eq!(t.turns, 1);
        assert_eq!(t.input_tokens, 100);
        assert_eq!(t.message_count, 1);

        // Second watched turn: append one more assistant message — again only
        // the appended delta is recorded.
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            f.write_all(format!("{ASSISTANT_LINE}\n").as_bytes())
                .unwrap();
        }
        let id2 = svc
            .token_usage
            .on_stop(&thread, Some("sess-1"), &payload, &TurnRecord::default())
            .await
            .unwrap();
        assert!(id2.is_some());
        let t = svc
            .token_usage_store
            .totals_for_thread(&thread_key)
            .await
            .unwrap();
        assert_eq!(t.turns, 2);
        assert_eq!(t.input_tokens, 200);
        assert_eq!(t.message_count, 2);

        // Stop with no new bytes → nothing recorded.
        let id3 = svc
            .token_usage
            .on_stop(&thread, Some("sess-1"), &payload, &TurnRecord::default())
            .await
            .unwrap();
        assert!(id3.is_none());
        assert_eq!(
            svc.token_usage_store
                .totals_for_thread(&thread_key)
                .await
                .unwrap()
                .turns,
            2
        );
    }

    #[tokio::test]
    async fn first_capture_seeds_cursor_without_ingesting_history() {
        // tsk142: on a fresh daemon attaching to an already-long session,
        // the first Stop has no stored cursor. It must SEED the cursor to the
        // current transcript end WITHOUT ingesting the prior history as one
        // giant turns:1 lump — we only attribute tokens spent while watching.
        let (svc, _dir, thread) = service_fixture().await;
        let tdir = tempfile::tempdir().unwrap();
        let path = tdir.path().join("session.jsonl");
        let thread_key = thread.to_string();
        let payload = format!(
            "{{\"transcript_path\":{:?},\"session_id\":\"sess-boot\"}}",
            path.to_string_lossy()
        );

        // Five prior assistant turns sat in the transcript before oxplow
        // attached.
        let mut history = String::new();
        for _ in 0..5 {
            history.push_str(ASSISTANT_LINE);
            history.push('\n');
        }
        std::fs::write(&path, &history).unwrap();

        // First Stop (no stored cursor): records nothing, just seeds.
        let id = svc
            .token_usage
            .on_stop(&thread, Some("sess-boot"), &payload, &TurnRecord::default())
            .await
            .unwrap();
        assert!(id.is_none(), "first capture must seed, not ingest history");
        let t = svc
            .token_usage_store
            .totals_for_thread(&thread_key)
            .await
            .unwrap();
        assert_eq!(t.turns, 0, "prior history must not be attributed");
        assert_eq!(t.input_tokens, 0);
        assert_eq!(t.message_count, 0);

        // A genuinely-new turn after we started watching IS recorded, and only
        // the new delta (one message), not the five prior.
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            f.write_all(format!("{ASSISTANT_LINE}\n").as_bytes())
                .unwrap();
        }
        let id2 = svc
            .token_usage
            .on_stop(&thread, Some("sess-boot"), &payload, &TurnRecord::default())
            .await
            .unwrap();
        assert!(id2.is_some());
        let t = svc
            .token_usage_store
            .totals_for_thread(&thread_key)
            .await
            .unwrap();
        assert_eq!(t.turns, 1, "only the new turn counts");
        assert_eq!(t.input_tokens, 100);
        assert_eq!(t.message_count, 1);
    }

    #[tokio::test]
    async fn on_stop_splits_a_multi_prompt_chunk_into_one_row_per_turn() {
        // tsk143: an effort spanning two prompts in a single Stop chunk must
        // yield two turn rows. (Bootstrap seeds first, so we land the prompts
        // on the second Stop.)
        let (svc, _dir, thread) = service_fixture().await;
        let tdir = tempfile::tempdir().unwrap();
        let path = tdir.path().join("session.jsonl");
        let thread_key = thread.to_string();
        let payload = format!(
            "{{\"transcript_path\":{:?},\"session_id\":\"sess-2p\"}}",
            path.to_string_lossy()
        );
        let user_a = serde_json::json!({"type":"user","message":{"content":"prompt A"}});
        let user_b = serde_json::json!({"type":"user","message":{"content":"prompt B"}});

        // Bootstrap: one assistant line already present; first Stop seeds only.
        std::fs::write(&path, format!("{ASSISTANT_LINE}\n")).unwrap();
        assert!(svc
            .token_usage
            .on_stop(&thread, Some("sess-2p"), &payload, &TurnRecord::default())
            .await
            .unwrap()
            .is_none());

        // Append two full turns, then Stop once: [A → turn][B → turn]. As
        // the reactor records it (tsk498): one `agent.turn.ended` is the
        // cause of both rows, and both must land.
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            f.write_all(
                format!("{user_a}\n{ASSISTANT_LINE}\n{user_b}\n{ASSISTANT_LINE}\n").as_bytes(),
            )
            .unwrap();
        }
        let rec = TurnRecord {
            turn_id: None,
            effort: None,
            agent_session: None,
            cause: Some("evt-stop-1".into()),
        };
        assert!(svc
            .token_usage
            .on_stop(&thread, Some("sess-2p"), &payload, &rec)
            .await
            .unwrap()
            .is_some());

        let t = svc
            .token_usage_store
            .totals_for_thread(&thread_key)
            .await
            .unwrap();
        assert_eq!(t.turns, 2, "two prompts → two turn rows");
        assert_eq!(t.message_count, 2);
    }

    #[tokio::test]
    async fn on_stop_no_transcript_path_is_noop() {
        let (svc, _dir, thread) = service_fixture().await;
        let id = svc
            .token_usage
            .on_stop(&thread, Some("sess-1"), "{}", &TurnRecord::default())
            .await
            .unwrap();
        assert!(id.is_none());
    }

    /// An agent's export, logged and counted (P10.M2).
    async fn report(svc: &crate::Services, thread: ThreadId, body: &[u8]) -> bool {
        let logged = svc.otlp_ingest.ingest(thread, None, body).await.unwrap();
        svc.event_pump.run_once().await.unwrap();
        logged
    }

    #[tokio::test]
    async fn a_reported_export_records_facts_once() {
        // tsk22: an OTLP metrics export lands per-kind token facts on
        // `oxplow.tokens`, and a retransmit of the identical export is a no-op.
        let (svc, _dir, thread) = service_fixture().await;
        let body = crate::otlp_tokens::encoded_claude_export("claude-opus-4-8", 100, 20);

        assert!(report(&svc, thread, &body).await);

        let measure = svc
            .fact_store
            .get_measure("oxplow.tokens")
            .await
            .unwrap()
            .unwrap();
        let facts = svc.fact_store.facts_for_measure(measure.id).await.unwrap();
        assert_eq!(facts.len(), 2);
        assert_eq!(facts.iter().map(|f| f.value).sum::<f64>(), 120.0);
        let input = facts
            .iter()
            .find(|f| {
                f.dims_json.as_deref()
                    == Some(
                        "{\"oxplow.model\":\"claude-opus-4-8\",\"oxplow.token_kind\":\"input\"}",
                    )
            })
            .expect("input-kind fact");
        assert_eq!(input.value, 100.0);
        assert_eq!(input.subject_ref.as_deref(), Some("model:claude-opus-4-8"));

        // Re-ingesting the identical export must not double-count (idempotency).
        assert!(!report(&svc, thread, &body).await);
        let facts_after = svc.fact_store.facts_for_measure(measure.id).await.unwrap();
        assert_eq!(facts_after.len(), 2, "retransmit is a no-op");

        // The producer specs re-aggregate the OTEL facts to the expected totals.
        svc.metrics.seed_catalog().await;
        let engine = crate::metric_engine::MetricEngine::new((*svc.fact_store).clone());
        for (key, expected) in [
            ("agent.tokens.total", 120.0),
            ("agent.tokens.input", 100.0),
            ("agent.tokens.output", 20.0),
        ] {
            let spec = svc.fact_store.get_spec(key).await.unwrap().unwrap();
            assert_eq!(
                engine.headline_for_spec(&spec).await.unwrap(),
                Some(expected),
                "{key}: spec headline over OTEL facts",
            );
        }
    }

    #[tokio::test]
    async fn a_reported_export_records_cache_facts_and_hit_ratio() {
        // tsk73: cache kinds land on `oxplow.cache_tokens` (NOT `oxplow.tokens`
        // — `agent.tokens.total` must keep meaning input+output), plus one
        // per-model `oxplow.cache_usage` ratio fact: num = cache_read, den =
        // prompt-side total (input + cache_read + cache_creation).
        let (svc, _dir, thread) = service_fixture().await;
        let body = crate::otlp_tokens::encoded_claude_export_with_cache(
            "claude-fable-5",
            100, // input
            20,  // output
            700, // cacheRead
            200, // cacheCreation
        );

        assert!(report(&svc, thread, &body).await);

        // input/output stay on oxplow.tokens — total is NOT cache-polluted.
        let tokens = svc
            .fact_store
            .get_measure("oxplow.tokens")
            .await
            .unwrap()
            .unwrap();
        let token_facts = svc.fact_store.facts_for_measure(tokens.id).await.unwrap();
        assert_eq!(token_facts.iter().map(|f| f.value).sum::<f64>(), 120.0);

        // Cache kinds on their own measure, sliced by token_kind.
        let cache = svc
            .fact_store
            .get_measure("oxplow.cache_tokens")
            .await
            .unwrap()
            .unwrap();
        let cache_facts = svc.fact_store.facts_for_measure(cache.id).await.unwrap();
        assert_eq!(cache_facts.len(), 2);
        assert_eq!(cache_facts.iter().map(|f| f.value).sum::<f64>(), 900.0);

        // The hit-ratio fact: 700 / (100 + 700 + 200) = 70%.
        let usage = svc
            .fact_store
            .get_measure("oxplow.cache_usage")
            .await
            .unwrap()
            .unwrap();
        let usage_facts = svc.fact_store.facts_for_measure(usage.id).await.unwrap();
        assert_eq!(usage_facts.len(), 1);
        assert_eq!(usage_facts[0].numerator, Some(700.0));
        assert_eq!(usage_facts[0].denominator, Some(1000.0));
        assert!((usage_facts[0].value - 70.0).abs() < 1e-9);

        // The specs read back: sums by kind + the cumulative ratio headline.
        svc.metrics.seed_catalog().await;
        let engine = crate::metric_engine::MetricEngine::new((*svc.fact_store).clone());
        for (key, expected) in [
            ("agent.tokens.total", 120.0),
            ("agent.tokens.cache_read", 700.0),
            ("agent.tokens.cache_creation", 200.0),
            ("agent.tokens.cache_hit_pct", 70.0),
        ] {
            let spec = svc.fact_store.get_spec(key).await.unwrap().unwrap();
            assert_eq!(
                engine.headline_for_spec(&spec).await.unwrap(),
                Some(expected),
                "{key}: spec headline",
            );
        }
    }

    #[tokio::test]
    async fn cache_free_export_emits_no_ratio_fact() {
        // An agent that reports no cache telemetry must read as "no data",
        // not a 0% point dragging the cumulative hit ratio down.
        let (svc, _dir, thread) = service_fixture().await;
        let body = crate::otlp_tokens::encoded_claude_export("claude-fable-5", 100, 20);
        report(&svc, thread, &body).await;
        let usage = svc
            .fact_store
            .get_measure("oxplow.cache_usage")
            .await
            .unwrap()
            .unwrap();
        assert!(svc
            .fact_store
            .facts_for_measure(usage.id)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn on_stop_projects_turn_facts_not_token_facts() {
        // tsk22 (transcript split): the Stop-hook transcript path now projects
        // only `oxplow.turn` facts — the `oxplow.tokens` facts come from the
        // OTEL producer (see `a_reported_export_records_facts_once`).
        let (svc, _dir, thread) = service_fixture().await;
        let tdir = tempfile::tempdir().unwrap();
        let path = tdir.path().join("session.jsonl");
        let payload = format!(
            "{{\"transcript_path\":{:?},\"session_id\":\"sess-m\"}}",
            path.to_string_lossy()
        );
        // Bootstrap (seed cursor, record nothing).
        std::fs::write(&path, format!("{ASSISTANT_LINE}\n")).unwrap();
        svc.token_usage
            .on_stop(&thread, Some("sess-m"), &payload, &TurnRecord::default())
            .await
            .unwrap();
        // Append one watched turn.
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            f.write_all(format!("{ASSISTANT_LINE}\n").as_bytes())
                .unwrap();
        }
        svc.token_usage
            .on_stop(&thread, Some("sess-m"), &payload, &TurnRecord::default())
            .await
            .unwrap();

        // NO token facts from the transcript path (OTEL owns them now).
        let tokens_measure = svc
            .fact_store
            .get_measure("oxplow.tokens")
            .await
            .unwrap()
            .unwrap();
        let token_facts = svc
            .fact_store
            .facts_for_measure(tokens_measure.id)
            .await
            .unwrap();
        assert!(
            token_facts.is_empty(),
            "transcript path no longer projects oxplow.tokens facts"
        );

        // …but a turn fact IS recorded, attributed to the thread.
        let turn_measure = svc
            .fact_store
            .get_measure("oxplow.turn")
            .await
            .unwrap()
            .unwrap();
        let turn_facts = svc
            .fact_store
            .facts_for_measure(turn_measure.id)
            .await
            .unwrap();
        assert_eq!(turn_facts.len(), 1);
        assert_eq!(turn_facts[0].value, 1.0, "one turn");
        assert_eq!(turn_facts[0].thread_id, Some(thread.value()));
        assert_eq!(
            turn_facts[0].subject_ref.as_deref(),
            Some("model:claude-opus-4-8")
        );

        // The `agent.turns` spec re-aggregates the turn fact.
        svc.metrics.seed_catalog().await;
        let engine = crate::metric_engine::MetricEngine::new((*svc.fact_store).clone());
        let spec = svc
            .fact_store
            .get_spec("agent.turns")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            engine.headline_for_spec(&spec).await.unwrap(),
            Some(1.0),
            "agent.turns headline over the turn fact",
        );
    }

    #[tokio::test]
    async fn on_stop_stamps_token_capture_with_the_open_effort() {
        // tsk37: the token fact-capture is stamped with the thread's single open
        // effort (the same resolution the run-ledger auto-claim uses), so
        // `captures_for_effort` — the T-D fact-attribution read — picks it up.
        use oxplow_domain::Timestamp;
        use oxplow_tasks::TaskId;
        use oxplow_tasks::TaskStore;
        use oxplow_tasks::{Task, TaskActorKind, TaskAuthor, TaskPriority, TaskStatus};
        let (svc, _dir, thread) = service_fixture().await;
        // One open effort on the thread → the unambiguous single-open case.
        let now = Timestamp::now();
        let task_id = svc
            .task_store
            .insert(&Task {
                id: TaskId::placeholder(),
                thread_id: Some(thread),
                parent_id: None,
                title: "t".into(),
                description: String::new(),
                status: TaskStatus::InProgress,
                priority: TaskPriority::Medium,
                sort_index: 0,
                created_by: TaskActorKind::User,
                created_at: now,
                updated_at: now,
                completed_at: None,
                deleted_at: None,
                note_count: 0,
                author: Some(TaskAuthor::User),
            })
            .await
            .unwrap();
        let effort = svc
            .effort_store
            .start(&work_item_ref(task_id), &thread, None)
            .await
            .unwrap();

        let tdir = tempfile::tempdir().unwrap();
        let path = tdir.path().join("session.jsonl");
        let payload = format!(
            "{{\"transcript_path\":{:?},\"session_id\":\"sess-e\"}}",
            path.to_string_lossy()
        );
        std::fs::write(&path, format!("{ASSISTANT_LINE}\n")).unwrap();
        svc.token_usage
            .on_stop(&thread, Some("sess-e"), &payload, &TurnRecord::default())
            .await
            .unwrap();
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            f.write_all(format!("{ASSISTANT_LINE}\n").as_bytes())
                .unwrap();
        }
        svc.token_usage
            .on_stop(&thread, Some("sess-e"), &payload, &TurnRecord::default())
            .await
            .unwrap();

        // The turn capture is attributed to the open effort (tsk22: the
        // transcript path projects turn facts; token facts ride the OTEL path).
        let caps = svc
            .fact_store
            .captures_for_effort(effort.id.value())
            .await
            .unwrap();
        assert!(
            !caps.is_empty(),
            "the turn capture is attributed to the open effort"
        );
        assert!(caps.iter().all(|c| c.effort_id == Some(effort.id.value())));
        // …and its facts are reachable through the fact-attribution read.
        let turn_measure = svc
            .fact_store
            .get_measure("oxplow.turn")
            .await
            .unwrap()
            .unwrap();
        let cap_ids: Vec<i64> = caps.iter().map(|c| c.id).collect();
        let facts = svc
            .fact_store
            .facts_for_captures(turn_measure.id, cap_ids)
            .await
            .unwrap();
        assert_eq!(facts.len(), 1, "one turn fact under the effort's capture");
    }
}
