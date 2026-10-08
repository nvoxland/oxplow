-- A thread is a line of the person's work; the agent slots on it are
-- `agent_session` rows (.context/data-model.md "agent_session"). Each
-- existing thread's one agent becomes one session — a chat for an ACP
-- thread, else a terminal — closed when its thread is closed or its
-- stream archived. Its turns and `agent.*` events carry it, and the
-- thread loses its agent columns.

CREATE TABLE agent_session (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id         INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    kind              TEXT NOT NULL CHECK (kind IN ('terminal', 'chat', 'action')),
    -- A harness registry key: no CHECK on purpose.
    harness           TEXT NOT NULL,
    acp_agent         TEXT,
    title             TEXT NOT NULL DEFAULT '',
    resume_session_id TEXT NOT NULL DEFAULT '',
    -- The host it runs on; NULL is the local machine.
    host              TEXT,
    opened_at         TEXT NOT NULL,
    closed_at         TEXT,
    closed_reason     TEXT CHECK (closed_reason IN ('closed', 'thread_closed', 'stream_archived')),
    updated_at        TEXT NOT NULL
) STRICT;
CREATE INDEX idx_agent_session_thread ON agent_session(thread_id, opened_at DESC);
CREATE INDEX idx_agent_session_open ON agent_session(thread_id) WHERE closed_at IS NULL;
CREATE INDEX idx_agent_session_resume ON agent_session(resume_session_id)
    WHERE resume_session_id <> '';

INSERT INTO agent_session
    (thread_id, kind, harness, acp_agent, resume_session_id, opened_at,
     closed_at, closed_reason, updated_at)
SELECT id,
       CASE agent WHEN 'acp' THEN 'chat' ELSE 'terminal' END,
       agent, acp_agent, resume_session_id, created_at,
       COALESCE(closed_at, archived_at),
       CASE WHEN closed_at IS NOT NULL THEN 'thread_closed'
            WHEN archived_at IS NOT NULL THEN 'stream_archived' END,
       updated_at
  FROM threads ORDER BY id;

-- NULL for a turn no session claims (a hook from an agent oxplow didn't start).
ALTER TABLE agent_turn ADD COLUMN agent_session_id INTEGER
    REFERENCES agent_session(id) ON DELETE SET NULL;
UPDATE agent_turn SET agent_session_id =
    (SELECT s.id FROM agent_session s WHERE s.thread_id = agent_turn.thread_id);
CREATE INDEX idx_agent_turn_open_session ON agent_turn(agent_session_id)
    WHERE ended_at IS NULL;

ALTER TABLE event_log ADD COLUMN agent_session_id INTEGER;
UPDATE event_log SET agent_session_id =
    (SELECT s.id FROM agent_session s WHERE s.thread_id = event_log.thread_id)
 WHERE type LIKE 'agent.%' AND thread_id IS NOT NULL;
CREATE INDEX idx_event_log_agent_session ON event_log(agent_session_id, seq)
    WHERE agent_session_id IS NOT NULL;

ALTER TABLE threads DROP COLUMN agent;
ALTER TABLE threads DROP COLUMN acp_agent;
ALTER TABLE threads DROP COLUMN resume_session_id;
ALTER TABLE threads DROP COLUMN pane_target;
