-- Every file a tool call names or reports, so a turn's changes can say
-- which of its calls claimed a file. agent_tool_call.path keeps the first
-- only.
--   basis  named (an edit tool named the file) or reported (the harness
--          reported a shell call changed it, Claude Code's bashEditDiff).
CREATE TABLE agent_tool_call_path (
    call_id INTEGER NOT NULL REFERENCES agent_tool_call(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    basis TEXT NOT NULL,
    PRIMARY KEY (call_id, path)
) WITHOUT ROWID;
CREATE INDEX idx_agent_tool_call_path_path ON agent_tool_call_path(path);

-- A turn's files are read by stream and capture time.
CREATE INDEX idx_file_snapshot_stream_captured ON file_snapshot(stream_id, captured_at);

-- The calls already recorded: their files, from the finish they project.
INSERT OR IGNORE INTO agent_tool_call_path (call_id, path, basis)
SELECT c.id, p.value, CASE c.kind WHEN 'edit' THEN 'named' ELSE 'reported' END
  FROM agent_tool_call c
  JOIN event_log e ON e.id = c.event_id
  JOIN json_each(e.payload, '$.paths') p
 WHERE c.kind IN ('edit', 'shell') AND e.payload_expired_at IS NULL
   AND json_type(e.payload, '$.paths') = 'array';
