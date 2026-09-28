-- ACP agents (tsk335): threads.agent gains 'acp', and an ACP thread names
-- which ACP agent it runs (acpAgents presets or the project's entries).
--
-- Same column swap as V32: SQLite can't alter a CHECK in place, and a
-- table rebuild would cascade-delete every child row (foreign_keys=ON).
-- v_thread reads the column, so it's dropped around the swap and
-- recreated with acp_agent.
DROP VIEW v_thread;

ALTER TABLE threads ADD COLUMN agent_next TEXT NOT NULL DEFAULT 'claude'
    CHECK (agent_next IN ('claude', 'codex', 'opencode', 'acp'));

UPDATE threads SET agent_next = agent;

ALTER TABLE threads DROP COLUMN agent;

ALTER TABLE threads RENAME COLUMN agent_next TO agent;

ALTER TABLE threads ADD COLUMN acp_agent TEXT;

CREATE VIEW v_thread AS
SELECT id, stream_id, title, status, agent, sort_index, created_at, updated_at,
       closed_at, archived_at, acp_agent
FROM threads;
