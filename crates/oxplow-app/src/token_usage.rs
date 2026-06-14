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
//! `None` (their session formats differ — opencode surfaces its own
//! `$cost` — and are wired later). Display is tokens-only for now; the
//! actual per-turn `model` is stored so cost can be layered on later.
//!
//! Mirrors the collection side-band (`collection.rs`): find open effort →
//! record → emit, best-effort. See `.context/agent-model.md` +
//! `.context/data-model.md`.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use oxplow_db::TaskEffortStore;
use oxplow_db::{
    NewAgentTokenUsage, SqliteTaskEffortStore, SqliteThreadStore, SqliteTokenUsageStore,
};
use oxplow_domain::stores::ThreadStore;
use oxplow_domain::{AgentKind, DomainError, ThreadId};

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

/// Captures per-turn token usage from the agent transcript on Stop.
#[derive(Clone)]
pub struct TokenUsageService {
    usage: Arc<SqliteTokenUsageStore>,
    efforts: Arc<SqliteTaskEffortStore>,
    threads: Arc<SqliteThreadStore>,
    events: EventBus,
}

impl TokenUsageService {
    pub fn new(
        usage: Arc<SqliteTokenUsageStore>,
        efforts: Arc<SqliteTaskEffortStore>,
        threads: Arc<SqliteThreadStore>,
        events: EventBus,
    ) -> Self {
        Self {
            usage,
            efforts,
            threads,
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

        let offset = self.usage.cursor(&session_key).await?.unwrap_or(0);
        let Some((tail, new_offset)) = read_complete_tail(&transcript_path, offset) else {
            return Ok(None);
        };

        let Some(delta) = parse_usage_delta(kind, &tail) else {
            // Nothing to record from this chunk (no usage / unsupported
            // agent), but the bytes are consumed — advance so we don't
            // re-scan them every Stop.
            self.usage.set_cursor(&session_key, new_offset).await?;
            return Ok(None);
        };

        let effort_id = self
            .efforts
            .find_open_for_thread(thread)
            .await?
            .map(|e| e.id.to_string());

        let id = self
            .usage
            .record(NewAgentTokenUsage {
                stream_id,
                thread_id: thread.to_string(),
                effort_id: effort_id.clone(),
                session_id: session_key.clone(),
                agent_kind: kind.as_str().to_string(),
                model: delta.model,
                input_tokens: delta.input_tokens,
                output_tokens: delta.output_tokens,
                cache_creation_input_tokens: delta.cache_creation_input_tokens,
                cache_read_input_tokens: delta.cache_read_input_tokens,
                message_count: delta.message_count,
            })
            .await?;
        self.usage.set_cursor(&session_key, new_offset).await?;
        self.events.emit(OxplowEvent::AgentTokenUsageChanged {
            thread_id: *thread,
            effort_id,
        });
        Ok(Some(id))
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

        // First turn: one assistant message.
        std::fs::write(&path, format!("{ASSISTANT_LINE}\n")).unwrap();
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

        // Second turn: append one more assistant message. The new row must
        // reflect ONLY the appended delta, not the whole file.
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
        // Each turn wrote a single-message delta of 100 input tokens.
        let t = svc
            .token_usage_store
            .totals_for_thread(&thread_key)
            .await
            .unwrap();
        assert_eq!(t.turns, 2);
        assert_eq!(t.input_tokens, 200);
        assert_eq!(t.message_count, 2);

        // Third Stop with no new bytes → nothing recorded.
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
    async fn on_stop_no_transcript_path_is_noop() {
        let (svc, _dir, thread) = service_fixture().await;
        let id = svc
            .token_usage
            .on_stop(&thread, Some("sess-1"), "{}")
            .await
            .unwrap();
        assert!(id.is_none());
    }
}
