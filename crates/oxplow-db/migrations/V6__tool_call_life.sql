-- A tool call's whole life: its row opens at the request and closes at the
-- finish.
--   call_id           the harness's id for the call.
--   started_at        when it was requested (NULL when no request was seen).
--   ended_at          when it finished, was denied, or its turn ended with
--                     it open.
--   duration_ms       how long it ran: the harness's own timing, else
--                     ended_at - started_at; NULL when it didn't finish.
--   state             running, finished, denied, abandoned (its turn or
--                     its session ended with it open).
--   request_event_id  the agent.tool.requested event the row opened with.
ALTER TABLE agent_tool_call ADD COLUMN call_id TEXT;
ALTER TABLE agent_tool_call ADD COLUMN started_at TEXT;
ALTER TABLE agent_tool_call ADD COLUMN ended_at TEXT;
ALTER TABLE agent_tool_call ADD COLUMN duration_ms INTEGER;
-- No CHECK on state: Rust owns its values.
ALTER TABLE agent_tool_call ADD COLUMN state TEXT NOT NULL DEFAULT 'finished';
ALTER TABLE agent_tool_call ADD COLUMN request_event_id TEXT;
CREATE UNIQUE INDEX idx_agent_tool_call_request ON agent_tool_call(request_event_id)
    WHERE request_event_id IS NOT NULL;
CREATE INDEX idx_agent_tool_call_running ON agent_tool_call(turn_id)
    WHERE state = 'running';

-- The rows already recorded are finished calls: each gets its request (the
-- finish event's cause) and the time between.
UPDATE agent_tool_call
   SET ended_at = at,
       request_event_id = (SELECT f.cause FROM event_log f WHERE f.id = agent_tool_call.event_id)
 WHERE event_id IS NOT NULL;
UPDATE agent_tool_call
   SET started_at = (SELECT r.at FROM event_log r WHERE r.id = agent_tool_call.request_event_id)
 WHERE request_event_id IS NOT NULL;
UPDATE agent_tool_call
   SET duration_ms = CAST(round((julianday(ended_at) - julianday(started_at)) * 86400000) AS INTEGER)
 WHERE started_at IS NOT NULL AND ended_at IS NOT NULL;

-- agent.tool.requested@3 / agent.tool.finished@3 → @4: the new fields are
-- optional, so the logged rows are the newest shape as they are.
UPDATE event_log SET v = 4
 WHERE type IN ('agent.tool.requested', 'agent.tool.finished') AND v = 3;
