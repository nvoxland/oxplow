-- Attribution past the door: the record carries the agent session a tool
-- call, a claimed or observed file and a reasoning claim belong to (two
-- sessions on one thread are told apart by their bearers), and a tool
-- call run inside a subagent carries which one. `shared` marks an
-- observed file that more than one session's turns could have changed.
-- Existing rows keep NULL: no session was recorded for them.
ALTER TABLE agent_tool_call ADD COLUMN agent_session_id INTEGER;
ALTER TABLE agent_tool_call ADD COLUMN subagent_id TEXT;
ALTER TABLE agent_tool_call ADD COLUMN subagent_kind TEXT;
ALTER TABLE effort_file ADD COLUMN agent_session_id INTEGER;
ALTER TABLE effort_file ADD COLUMN shared INTEGER NOT NULL DEFAULT 0;
ALTER TABLE claim ADD COLUMN agent_session_id INTEGER;
