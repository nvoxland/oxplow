-- Agent token usage (tsk104) — per-turn token accounting parsed from the
-- agent's session transcript.
--
-- The PTY is opaque, but the hook payload oxplow receives on Stop carries
-- `transcript_path` (the Claude session JSONL), whose assistant messages
-- each carry a `usage` block (input / output / cache-creation / cache-read
-- tokens) plus `model`. On Stop the runtime sums the NEW usage records
-- since the last Stop and writes one row here, attributed to the open
-- effort (nullable — a Stop can land with no open effort) and the thread.
-- Provenance is always `observed`: oxplow read the transcript directly.
-- `model` is the actual per-turn model so $ cost can be layered on later;
-- display is tokens-only for now. See `.context/agent-model.md` (token
-- usage capture) and `.context/data-model.md`.

CREATE TABLE agent_token_usage (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    effort_id INTEGER REFERENCES task_effort(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    agent_kind TEXT NOT NULL,
    model TEXT,
    input_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    cache_creation_input_tokens INTEGER NOT NULL DEFAULT 0,
    cache_read_input_tokens INTEGER NOT NULL DEFAULT 0,
    message_count INTEGER NOT NULL DEFAULT 0,
    provenance TEXT NOT NULL CHECK (provenance IN ('observed')),
    recorded_at TEXT NOT NULL
);

CREATE INDEX idx_agent_token_usage_effort
    ON agent_token_usage(effort_id, recorded_at DESC);
CREATE INDEX idx_agent_token_usage_thread
    ON agent_token_usage(thread_id, recorded_at DESC);

-- Per-session read cursor so successive Stops only sum the NEW tail of the
-- transcript instead of re-summing the whole file. Persisted (not just
-- in-memory) so a daemon restart doesn't double-count already-recorded
-- usage. Keyed by session_id (1:1 with the transcript file for Claude).
CREATE TABLE agent_token_cursor (
    session_id TEXT PRIMARY KEY,
    byte_offset INTEGER NOT NULL,
    updated_at TEXT NOT NULL
);
