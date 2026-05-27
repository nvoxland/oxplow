-- Effort-scoped collection: structured, agent-or-tool-reported observations
-- attached to a `task_effort` (test runs, diff coverage, …). Modeled on
-- `code_quality_finding` (kind + metric + payload) with the `page_ref`
-- freshness-pin columns so each observation knows how out-of-date it is.
--
-- Provenance is the spine: `observed` = oxplow saw it directly (the
-- PostToolUse Bash hook, or oxplow parsing a coverage report itself);
-- `asserted` = the agent reported it via MCP and we can't independently
-- verify it.
--
-- `effort_id` is NOT NULL with ON DELETE CASCADE: an observation only has
-- meaning inside its effort's start/end snapshot bracket (e.g. diff-coverage
-- intersects against the effort's changed lines), so it dies with the effort.
--
-- Freshness pin (same shape as `task_effort_file` / `page_ref`, see V20):
--   * `local_snapshot_id`   — the snapshot the observation was captured
--     against (the effort's end snapshot for diff-coverage). Nullable —
--     a `test-run` recorded from the hook may have no snapshot.
--   * `closest_git_version` — closest git commit at capture time.
--   * `git_version_exact`   — 1 when the local snapshot is byte-equal to
--     that commit; the UI flips a staleness marker when this is 0 / the
--     pin falls behind HEAD.

CREATE TABLE effort_observation (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id TEXT NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    effort_id TEXT NOT NULL REFERENCES task_effort(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    provenance TEXT NOT NULL CHECK (provenance IN ('observed', 'asserted')),
    source TEXT NOT NULL,
    metric_value REAL,
    payload_json TEXT,
    local_snapshot_id INTEGER,
    closest_git_version TEXT,
    git_version_exact INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_effort_observation_effort
    ON effort_observation(effort_id, kind, created_at DESC);
CREATE INDEX idx_effort_observation_stream
    ON effort_observation(stream_id);
