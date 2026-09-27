-- Agent tool calls (tsk296): every PostToolUse the agent fires, persisted
-- (the hook ring is in-memory and capped). Feeds context-read and struggle
-- views for review.
CREATE TABLE agent_tool_call (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    effort_id INTEGER REFERENCES task_effort(id) ON DELETE SET NULL,
    tool TEXT NOT NULL,
    -- Repo-relative when inside the project; NULL for tools without a path.
    path TEXT,
    -- Short context: the Bash command (truncated), a Grep pattern, …
    detail TEXT,
    -- 1 ok, 0 failed, NULL unknown (Claude's Bash response often has no exit code).
    ok INTEGER,
    at TEXT NOT NULL
);
CREATE INDEX idx_agent_tool_call_effort ON agent_tool_call(effort_id, tool);

CREATE VIEW v_tool_call AS
SELECT id, thread_id, effort_id, tool, path, detail, ok, at FROM agent_tool_call;

CREATE VIEW v_context_read AS
SELECT id, thread_id, effort_id, path, at
FROM agent_tool_call
WHERE tool = 'Read' AND path LIKE '.context/%.md';

-- Where the agent had trouble in an effort: a file edited 5+ times, or 3+
-- failed commands. Thresholds are deliberately blunt; lenses can refine by
-- querying v_tool_call directly.
CREATE VIEW v_struggle AS
SELECT effort_id, thread_id, 'repeated_edits' AS kind, path AS subject, count(*) AS count
FROM agent_tool_call
WHERE tool IN ('Edit', 'Write', 'MultiEdit', 'NotebookEdit')
  AND path IS NOT NULL AND effort_id IS NOT NULL
GROUP BY effort_id, thread_id, path
HAVING count(*) >= 5
UNION ALL
SELECT effort_id, thread_id, 'failed_commands' AS kind, 'Bash' AS subject, count(*) AS count
FROM agent_tool_call
WHERE tool = 'Bash' AND ok = 0 AND effort_id IS NOT NULL
GROUP BY effort_id, thread_id
HAVING count(*) >= 3;
