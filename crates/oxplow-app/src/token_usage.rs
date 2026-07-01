//! Agent token-usage capture (tsk104).
//!
//! The PTY is opaque, but the hook payload oxplow receives on Stop carries
//! `transcript_path` (the agent session JSONL). For Claude, each
//! `type=="assistant"` line carries a `message.usage` block
//! (input / output / cache-creation / cache-read tokens) plus
//! `message.model`. On Stop we sum the NEW usage records since the last
//! Stop (offset-tracked via a persisted per-session cursor, so we never
//! re-sum the whole file or double-count across restarts) and persist one
//! row attributed to the open effort + thread. Provenance is always
//! `observed` — oxplow read the transcript directly.
//!
//! Pluggable per agent kind: Claude is implemented; Codex/Opencode return
//! `None` (their session formats differ — and are wired later). We track
//! token counts only; oxplow deliberately does not derive a USD price (rates
//! move and a stale price table is worse than none). The per-turn `model` is
//! stored so usage can be sliced by model.
//!
//! Mirrors the collection side-band (`collection.rs`): find open effort →
//! record → emit, best-effort. See `.context/agent-model.md` +
//! `.context/data-model.md`.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxplow_db::TaskEffortStore;
use oxplow_db::{
    NewAgentTokenUsage, NewFact, NewMetricCapture, SqliteFactStore, SqliteTaskEffortStore,
    SqliteThreadStore, SqliteTokenUsageStore,
};
use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{AgentKind, DomainError, StreamId, ThreadId};

use crate::events::{EventBus, OxplowEvent};

/// Summed usage across a chunk of transcript (one Stop's delta).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageDelta {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_creation_input_tokens: i64,
    pub cache_read_input_tokens: i64,
    pub message_count: i64,
    /// The model of the last assistant message in the chunk.
    pub model: Option<String>,
}

impl UsageDelta {
    fn is_empty(&self) -> bool {
        self.message_count == 0
    }
}

/// Sum the `usage` blocks across every `type=="assistant"` line in a chunk
/// of Claude transcript JSONL. Blank and malformed lines are skipped, so a
/// partially-written tail line never poisons the sum.
pub fn parse_claude_usage(content: &str) -> UsageDelta {
    let mut d = UsageDelta::default();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        let Some(msg) = v.get("message") else {
            continue;
        };
        let Some(usage) = msg.get("usage") else {
            continue;
        };
        let get = |k: &str| usage.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
        d.input_tokens += get("input_tokens");
        d.output_tokens += get("output_tokens");
        d.cache_creation_input_tokens += get("cache_creation_input_tokens");
        d.cache_read_input_tokens += get("cache_read_input_tokens");
        d.message_count += 1;
        if let Some(m) = msg.get("model").and_then(|m| m.as_str()) {
            d.model = Some(m.to_string());
        }
    }
    d
}

/// Pluggable per-agent-kind usage parser. Returns `None` when there is
/// nothing to record (no usage in the chunk, or an agent kind whose
/// transcript format isn't parsed yet).
pub fn parse_usage_delta(kind: AgentKind, content: &str) -> Option<UsageDelta> {
    match kind {
        AgentKind::Claude => {
            let d = parse_claude_usage(content);
            (!d.is_empty()).then_some(d)
        }
        // Codex / opencode session formats differ (opencode surfaces its
        // own $cost). Stubbed until their parsers land.
        AgentKind::Codex | AgentKind::Opencode => None,
    }
}

/// One agent turn within a transcript chunk: the human-authored prompt that
/// opened it (when present) plus the summed usage of the assistant messages
/// that answered it (tsk143). A "turn" begins at a genuine user prompt and
/// runs until the next one; assistant lines accumulate into the current turn.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Turn {
    /// The opening user prompt text, or `None` for an assistant continuation
    /// with no fresh prompt at the head of the chunk.
    pub prompt: Option<String>,
    pub usage: UsageDelta,
}

impl Turn {
    /// A turn is worth recording if it captured either a prompt or usage.
    fn is_recordable(&self) -> bool {
        self.prompt.is_some() || !self.usage.is_empty()
    }
}

/// Extract the human-authored prompt text from a Claude `type=="user"`
/// message, or `None` when the line is not a genuine user prompt. Claude
/// transcripts reuse `type=="user"` for two things: the actual prompt the
/// human typed, and tool-result continuations the harness injects. We only
/// want the former — so a user message whose content is exclusively
/// tool_result blocks (no text) is NOT a prompt and returns `None`. String
/// content is taken verbatim; array content joins its `text` blocks.
fn extract_user_prompt(msg: &serde_json::Value) -> Option<String> {
    let content = msg.get("content")?;
    if let Some(s) = content.as_str() {
        let s = s.trim();
        return (!s.is_empty()).then(|| s.to_string());
    }
    let arr = content.as_array()?;
    let mut parts = Vec::new();
    for block in arr {
        if block.get("type").and_then(|t| t.as_str()) == Some("text") {
            if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                let t = t.trim();
                if !t.is_empty() {
                    parts.push(t.to_string());
                }
            }
        }
    }
    (!parts.is_empty()).then(|| parts.join("\n"))
}

/// Split a Claude transcript chunk into per-turn rows. Each genuine user
/// prompt opens a new turn; assistant `usage` blocks accumulate into the
/// current turn; tool-result user messages are folded into the current turn
/// (they are not fresh prompts). Assistant lines that precede any prompt in
/// the chunk form a leading prompt-less turn.
pub fn parse_claude_turns(content: &str) -> Vec<Turn> {
    let mut turns: Vec<Turn> = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let kind = v.get("type").and_then(|t| t.as_str());
        let Some(msg) = v.get("message") else {
            continue;
        };
        match kind {
            Some("user") => {
                if let Some(prompt) = extract_user_prompt(msg) {
                    turns.push(Turn {
                        prompt: Some(prompt),
                        usage: UsageDelta::default(),
                    });
                }
                // tool_result-only user message → not a fresh turn; skip.
            }
            Some("assistant") => {
                let Some(usage) = msg.get("usage") else {
                    continue;
                };
                if turns.is_empty() {
                    turns.push(Turn::default());
                }
                let d = &mut turns.last_mut().expect("just pushed").usage;
                let get = |k: &str| usage.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
                d.input_tokens += get("input_tokens");
                d.output_tokens += get("output_tokens");
                d.cache_creation_input_tokens += get("cache_creation_input_tokens");
                d.cache_read_input_tokens += get("cache_read_input_tokens");
                d.message_count += 1;
                if let Some(m) = msg.get("model").and_then(|m| m.as_str()) {
                    d.model = Some(m.to_string());
                }
            }
            _ => {}
        }
    }
    turns
}

/// Pluggable per-agent-kind turn parser (tsk143). Returns the recordable
/// turns in this chunk — empty when there is nothing to persist (no usage
/// and no prompt, or an agent kind whose transcript format isn't parsed).
pub fn parse_turns(kind: AgentKind, content: &str) -> Vec<Turn> {
    match kind {
        AgentKind::Claude => parse_claude_turns(content)
            .into_iter()
            .filter(Turn::is_recordable)
            .collect(),
        // Codex / opencode session formats differ. Stubbed until their
        // parsers land (mirrors `parse_usage_delta`).
        AgentKind::Codex | AgentKind::Opencode => Vec::new(),
    }
}

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

/// Per-(stop, model) token totals, accumulated across the turns in one Stop so
/// the metric substrate gets one sample per model rather than one per turn.
#[derive(Default)]
struct TokenAgg {
    input: i64,
    output: i64,
    turns: i64,
}

/// Captures per-turn token usage from the agent transcript on Stop.
#[derive(Clone)]
pub struct TokenUsageService {
    usage: Arc<SqliteTokenUsageStore>,
    efforts: Arc<SqliteTaskEffortStore>,
    threads: Arc<SqliteThreadStore>,
    /// Durable fact layer (epic tsk12): per-kind token totals land as facts
    /// on the `oxplow.tokens` measure (the legacy sample write is gone, T-E2).
    facts: Arc<SqliteFactStore>,
    events: EventBus,
}

impl TokenUsageService {
    pub fn new(
        usage: Arc<SqliteTokenUsageStore>,
        efforts: Arc<SqliteTaskEffortStore>,
        threads: Arc<SqliteThreadStore>,
        facts: Arc<SqliteFactStore>,
        events: EventBus,
    ) -> Self {
        Self {
            usage,
            efforts,
            threads,
            facts,
            events,
        }
    }

    /// On Stop: parse the transcript tail since the last cursor, sum usage,
    /// and persist one row attributed to the open effort + thread. Returns
    /// `Ok(Some(id))` when a row was written, `Ok(None)` when there was
    /// nothing to record (no transcript_path, no thread, no new usage, or a
    /// non-Claude agent). Best-effort — the caller treats errors as
    /// non-fatal so a parse hiccup never blocks the hook.
    pub async fn on_stop(
        &self,
        thread: &ThreadId,
        session_id: Option<&str>,
        payload_json: &str,
    ) -> Result<Option<i64>, DomainError> {
        let Some(transcript_path) = extract_transcript_path(payload_json) else {
            return Ok(None);
        };
        let Some(thread_row) = self.threads.get(thread).await? else {
            return Ok(None);
        };
        let kind = thread_row.agent;
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

        let turns = parse_turns(kind, &tail);
        if turns.is_empty() {
            // Nothing to record from this chunk (no usage / no prompt /
            // unsupported agent), but the bytes are consumed — advance so we
            // don't re-scan them every Stop.
            self.usage.set_cursor(&session_key, new_offset).await?;
            return Ok(None);
        }

        // Attribute tokens to the effort only when unambiguous; under parallel
        // sub-agents (two open efforts) the turn isn't a single effort's, so it
        // stays unattributed rather than guessing (tsk263).
        let open_effort = self.efforts.find_single_open_for_thread(thread).await?;
        let effort_id = open_effort.as_ref().map(|e| e.id.to_string());
        // The i64 form stamps the fact-capture so `captures_for_effort` (the T-D
        // fact-attribution read) attributes the token facts (tsk37).
        let effort_val = open_effort.as_ref().map(|e| e.id.value());

        // One row per turn — each carrying its opening prompt, model, and the
        // usage of the assistant messages that answered it (tsk143). While we
        // record, accumulate per-model totals for the metric projection.
        let mut last_id = None;
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
            let id = self
                .usage
                .record(NewAgentTokenUsage {
                    stream_id: stream_id.clone(),
                    thread_id: thread.to_string(),
                    effort_id: effort_id.clone(),
                    session_id: session_key.clone(),
                    agent_kind: kind.as_str().to_string(),
                    model: turn.usage.model,
                    prompt: turn.prompt,
                    input_tokens: input,
                    output_tokens: output,
                    cache_creation_input_tokens: cc,
                    cache_read_input_tokens: cr,
                    message_count: turn.usage.message_count,
                })
                .await?;
            last_id = Some(id);
            let agg = by_model.entry(model_key).or_default();
            agg.input += input;
            agg.output += output;
            agg.turns += 1;
        }
        self.usage.set_cursor(&session_key, new_offset).await?;
        self.events.emit(OxplowEvent::AgentTokenUsageChanged {
            thread_id: *thread,
            effort_id,
        });
        // Project token samples into the unified substrate (best-effort).
        self.project_token_metrics(thread, &stream_id, &by_model, effort_val)
            .await;
        Ok(last_id)
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
    ) {
        if by_model.is_empty() {
            return;
        }
        let Some(stream_val) = StreamId::try_from_str(stream_id).map(|s| s.value()) else {
            return;
        };
        if let Err(e) = self
            .record_token_metrics(thread, stream_val, by_model, effort_val)
            .await
        {
            tracing::warn!(error = %e, "failed to project token usage into metric substrate");
            return;
        }
        self.events.emit(OxplowEvent::MetricSamplesChanged {
            stream_id: StreamId::new(stream_val),
        });
    }

    async fn record_token_metrics(
        &self,
        thread: &ThreadId,
        stream_val: i64,
        by_model: &std::collections::HashMap<String, TokenAgg>,
        effort_val: Option<i64>,
    ) -> Result<(), DomainError> {
        // The durable facts (epic tsk12; the legacy run/sample writes are gone,
        // T-E2): PER-KIND token facts on the `oxplow.tokens` measure — one
        // input + one output fact per model, sliced by the `oxplow.token_kind`
        // conformed dimension — plus a turn fact on `oxplow.turn`, under one
        // capture carrying the spine. The `agent.tokens.total` spec sums both
        // kinds; the input/output specs filter by `token_kind`. Tokens/turns
        // are additive event measures; model is a conformed dimension.
        let tokens_measure = self.facts.get_measure("oxplow.tokens").await?;
        let turn_measure = self.facts.get_measure("oxplow.turn").await?;
        if tokens_measure.is_some() || turn_measure.is_some() {
            let mut facts = Vec::new();
            for (model, agg) in by_model {
                if let Some(m) = &tokens_measure {
                    for (kind, value) in [("input", agg.input), ("output", agg.output)] {
                        if value == 0 {
                            continue;
                        }
                        facts.push(NewFact {
                            subject_kind: Some("model".into()),
                            subject_ref: Some(format!("model:{model}")),
                            // json! (not format!) — the model id comes verbatim
                            // from external session JSONL; a quote/backslash in
                            // it must not poison the dims JSON (tsk46).
                            dims_json: Some(
                                serde_json::json!({
                                    "oxplow.model": model,
                                    "oxplow.token_kind": kind,
                                })
                                .to_string(),
                            ),
                            ..NewFact::new(m.id, value as f64)
                        });
                    }
                }
                if let Some(tm) = &turn_measure {
                    if agg.turns > 0 {
                        facts.push(NewFact {
                            subject_kind: Some("model".into()),
                            subject_ref: Some(format!("model:{model}")),
                            dims_json: Some(
                                serde_json::json!({ "oxplow.model": model }).to_string(),
                            ),
                            ..NewFact::new(tm.id, agg.turns as f64)
                        });
                    }
                }
            }
            if !facts.is_empty() {
                let mut capture = NewMetricCapture::done(stream_val, "token-parse", "token-parse");
                capture.thread_id = Some(thread.value());
                capture.trigger = Some("continuous".into());
                capture.effort_id = effort_val;
                self.facts.record_facts(capture, facts).await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const ASSISTANT_LINE: &str = r#"{"type":"assistant","message":{"model":"claude-opus-4-8","usage":{"input_tokens":100,"output_tokens":20,"cache_creation_input_tokens":50,"cache_read_input_tokens":200}}}"#;

    #[test]
    fn parse_claude_usage_sums_assistant_lines_and_skips_others() {
        let content = format!(
            "{ASSISTANT_LINE}\n\
             {{\"type\":\"user\",\"message\":{{\"content\":\"hi\"}}}}\n\
             not json at all\n\
             {ASSISTANT_LINE}\n"
        );
        let d = parse_claude_usage(&content);
        assert_eq!(d.message_count, 2);
        assert_eq!(d.input_tokens, 200);
        assert_eq!(d.output_tokens, 40);
        assert_eq!(d.cache_creation_input_tokens, 100);
        assert_eq!(d.cache_read_input_tokens, 400);
        assert_eq!(d.model.as_deref(), Some("claude-opus-4-8"));
    }

    // A genuine user prompt line (string content).
    fn user_line(text: &str) -> String {
        serde_json::json!({"type": "user", "message": {"content": text}}).to_string()
    }

    #[test]
    fn parse_claude_turns_splits_one_turn_per_user_prompt() {
        // [prompt A → assistant turn][prompt B → assistant turn]
        let content = format!(
            "{}\n{ASSISTANT_LINE}\n{}\n{ASSISTANT_LINE}\n",
            user_line("prompt A"),
            user_line("prompt B"),
        );
        let turns = parse_claude_turns(&content);
        assert_eq!(turns.len(), 2);
        assert_eq!(turns[0].prompt.as_deref(), Some("prompt A"));
        assert_eq!(turns[0].usage.message_count, 1);
        assert_eq!(turns[0].usage.input_tokens, 100);
        assert_eq!(turns[1].prompt.as_deref(), Some("prompt B"));
        assert_eq!(turns[1].usage.message_count, 1);
        assert_eq!(turns[1].usage.model.as_deref(), Some("claude-opus-4-8"));
    }

    #[test]
    fn parse_turns_via_kind_carries_each_prompt() {
        let content = format!(
            "{}\n{ASSISTANT_LINE}\n{}\n{ASSISTANT_LINE}\n",
            user_line("prompt A"),
            user_line("prompt B"),
        );
        let turns = parse_turns(AgentKind::Claude, &content);
        let prompts: Vec<_> = turns.iter().map(|t| t.prompt.as_deref()).collect();
        assert_eq!(prompts, vec![Some("prompt A"), Some("prompt B")]);
        // Non-Claude agents are stubbed.
        assert!(parse_turns(AgentKind::Codex, &content).is_empty());
    }

    #[test]
    fn tool_result_user_messages_do_not_open_a_turn() {
        // A real prompt, then an assistant message, then a tool_result user
        // message (the harness continuation), then another assistant message.
        // The tool_result must NOT start a second turn — both assistant
        // messages fold into the single real-prompt turn.
        let tool_result = serde_json::json!({
            "type": "user",
            "message": {"content": [{"type": "tool_result", "content": "ok"}]}
        })
        .to_string();
        let content = format!(
            "{}\n{ASSISTANT_LINE}\n{tool_result}\n{ASSISTANT_LINE}\n",
            user_line("real prompt"),
        );
        let turns = parse_claude_turns(&content);
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].prompt.as_deref(), Some("real prompt"));
        assert_eq!(turns[0].usage.message_count, 2);
    }

    #[test]
    fn extract_user_prompt_joins_text_blocks_and_skips_tool_results() {
        let arr = serde_json::json!({"content": [
            {"type": "text", "text": "hello"},
            {"type": "tool_result", "content": "ignored"},
            {"type": "text", "text": "world"},
        ]});
        assert_eq!(extract_user_prompt(&arr).as_deref(), Some("hello\nworld"));
        let only_tool = serde_json::json!({"content": [{"type": "tool_result"}]});
        assert!(extract_user_prompt(&only_tool).is_none());
        let empty = serde_json::json!({"content": "   "});
        assert!(extract_user_prompt(&empty).is_none());
    }

    #[test]
    fn parse_usage_delta_is_pluggable_per_agent() {
        assert!(parse_usage_delta(AgentKind::Claude, ASSISTANT_LINE).is_some());
        // No usage at all → None even for Claude.
        assert!(parse_usage_delta(AgentKind::Claude, "{\"type\":\"user\"}\n").is_none());
        // Other agents are stubbed.
        assert!(parse_usage_delta(AgentKind::Codex, ASSISTANT_LINE).is_none());
        assert!(parse_usage_delta(AgentKind::Opencode, ASSISTANT_LINE).is_none());
    }

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

    fn init_git_repo(dir: &Path) {
        let repo = git2::Repository::init(dir).unwrap();
        let mut config = repo.config().unwrap();
        config.set_str("user.name", "test").unwrap();
        config.set_str("user.email", "test@example.com").unwrap();
        let sig = repo.signature().unwrap();
        let tree_id = repo.index().unwrap().write_tree().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .unwrap();
    }

    /// Build a real in-memory `Services` over a fresh git repo, seed a
    /// primary stream + a Claude thread, and return the service + thread id.
    /// Token usage is attributed to the thread (no open effort created — the
    /// effort-attribution path is covered by the store's `totals_for_effort`
    /// tests).
    async fn service_fixture() -> (std::sync::Arc<crate::Services>, tempfile::TempDir, ThreadId) {
        let dir = tempfile::tempdir().unwrap();
        init_git_repo(dir.path());
        let svc = std::sync::Arc::new(crate::Services::in_memory(dir.path()).unwrap());
        let stream = svc.streams.ensure_primary().await.unwrap();
        let thread = svc
            .threads
            .create(&stream.id, "T", "working", AgentKind::Claude)
            .await
            .unwrap();
        (svc, dir, thread.id)
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
            .on_stop(&thread, Some("sess-1"), &payload)
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
            .on_stop(&thread, Some("sess-1"), &payload)
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
            .on_stop(&thread, Some("sess-1"), &payload)
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
            .on_stop(&thread, Some("sess-1"), &payload)
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
            .on_stop(&thread, Some("sess-boot"), &payload)
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
            .on_stop(&thread, Some("sess-boot"), &payload)
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
            .on_stop(&thread, Some("sess-2p"), &payload)
            .await
            .unwrap()
            .is_none());

        // Append two full turns, then Stop once: [A → turn][B → turn].
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
        assert!(svc
            .token_usage
            .on_stop(&thread, Some("sess-2p"), &payload)
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
            .on_stop(&thread, Some("sess-1"), "{}")
            .await
            .unwrap();
        assert!(id.is_none());
    }

    #[tokio::test]
    async fn on_stop_projects_token_metrics() {
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
            .on_stop(&thread, Some("sess-m"), &payload)
            .await
            .unwrap();
        // Append one watched turn (input 100, output 20, model opus).
        {
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .unwrap();
            f.write_all(format!("{ASSISTANT_LINE}\n").as_bytes())
                .unwrap();
        }
        svc.token_usage
            .on_stop(&thread, Some("sess-m"), &payload)
            .await
            .unwrap();

        // The durable fact layer (epic tsk12; the legacy samples are gone,
        // T-E2): PER-KIND facts on the `oxplow.tokens` measure — input +
        // output, sliced by oxplow.token_kind — so the total spec sums both
        // and input/output specs filter by kind.
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
        assert_eq!(token_facts.len(), 2, "one input + one output fact");
        let sum: f64 = token_facts.iter().map(|f| f.value).sum();
        assert_eq!(sum, 120.0, "input 100 + output 20 = total");
        let input_fact = token_facts
            .iter()
            .find(|f| {
                f.dims_json.as_deref()
                    == Some(
                        "{\"oxplow.model\":\"claude-opus-4-8\",\"oxplow.token_kind\":\"input\"}",
                    )
            })
            .expect("input-kind fact");
        assert_eq!(input_fact.value, 100.0);
        assert_eq!(
            input_fact.subject_ref.as_deref(),
            Some("model:claude-opus-4-8")
        );
        assert_eq!(input_fact.thread_id, Some(thread.value()));

        // …and a turn fact on the `oxplow.turn` measure.
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

        // Keystone: the producer specs re-aggregate the facts to the baked totals
        // through the engine (the read-flip, tsk26, can then serve them).
        svc.metrics.seed_catalog().await;
        let engine = crate::metric_engine::MetricEngine::new((*svc.fact_store).clone());
        for (key, expected) in [
            ("agent.tokens.total", 120.0),
            ("agent.tokens.input", 100.0),
            ("agent.tokens.output", 20.0),
            ("agent.turns", 1.0),
        ] {
            let spec = svc.fact_store.get_spec(key).await.unwrap().unwrap();
            assert_eq!(
                engine.headline_for_spec(&spec).await.unwrap(),
                Some(expected),
                "{key}: spec headline over facts == baked total",
            );
        }
    }

    #[tokio::test]
    async fn on_stop_stamps_token_capture_with_the_open_effort() {
        // tsk37: the token fact-capture is stamped with the thread's single open
        // effort (the same resolution the run-ledger auto-claim uses), so
        // `captures_for_effort` — the T-D fact-attribution read — picks it up.
        use oxplow_domain::stores::TaskStore;
        use oxplow_domain::{
            Task, TaskActorKind, TaskAuthor, TaskId, TaskPriority, TaskStatus, Timestamp,
        };
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
            .start(task_id, &thread, None)
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
            .on_stop(&thread, Some("sess-e"), &payload)
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
            .on_stop(&thread, Some("sess-e"), &payload)
            .await
            .unwrap();

        // The token capture is attributed to the open effort.
        let caps = svc
            .fact_store
            .captures_for_effort(effort.id.value())
            .await
            .unwrap();
        assert!(
            !caps.is_empty(),
            "the token capture is attributed to the open effort"
        );
        assert!(caps.iter().all(|c| c.effort_id == Some(effort.id.value())));
        // …and its facts are reachable through the fact-attribution read.
        let tokens_measure = svc
            .fact_store
            .get_measure("oxplow.tokens")
            .await
            .unwrap()
            .unwrap();
        let cap_ids: Vec<i64> = caps.iter().map(|c| c.id).collect();
        let facts = svc
            .fact_store
            .facts_for_captures(tokens_measure.id, cap_ids)
            .await
            .unwrap();
        assert_eq!(
            facts.len(),
            2,
            "input + output token facts under the effort's capture"
        );
    }
}
