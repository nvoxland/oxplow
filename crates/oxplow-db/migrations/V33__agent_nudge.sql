-- Persisted agent nudges: the informational steers oxplow surfaces to the
-- agent from the PostToolUse hook (the report-less-test-run nudge and the
-- commit-hygiene nudge — see `crates/oxplow-app/src/collection.rs`
-- `on_post_tool_use`). These were previously fully ephemeral: the string was
-- returned to the control-plane, forwarded via `additionalContext`, then
-- lost. Persisting them gives a reviewer/human-facing record of "what oxplow
-- told the agent this effort."
--
-- Scope:
--   * `thread_id` is NOT NULL with ON DELETE CASCADE — every nudge fires
--     within a thread.
--   * `effort_id` is NULLABLE with ON DELETE CASCADE — nudges fire against
--     the open effort today, but some kinds may fire with no open effort
--     (thread-scoped only). Cascades with the effort like `effort_observation`
--     when present.
--
-- `kind` is open-ended (e.g. `report-less-run` | `commit-hygiene` |
-- `configure`); `trigger` is the bash command (or commit sha) that caused it.
-- One-shot dedup lives in the service (in-memory, keyed by effort/commit), so
-- the table only ever sees nudges that actually fired.

CREATE TABLE agent_nudge (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    effort_id INTEGER REFERENCES task_effort(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    message TEXT NOT NULL,
    trigger TEXT,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_agent_nudge_effort
    ON agent_nudge(effort_id, created_at DESC);
CREATE INDEX idx_agent_nudge_thread
    ON agent_nudge(thread_id, created_at DESC);
