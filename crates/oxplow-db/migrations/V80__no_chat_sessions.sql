-- ACP chat sessions are gone: a session is a terminal (or, later, a
-- one-shot action) and runs a harness the project has. Any chat session
-- still open can no longer be launched, so it is closed with its open
-- turns; the `acp_agent` column and the `chat` kind leave the table
-- (.context/data-model.md "agent_session").

UPDATE agent_turn
   SET ended_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), answer = 'session closed'
 WHERE ended_at IS NULL
   AND agent_session_id IN (SELECT id FROM agent_session WHERE kind = 'chat');
UPDATE agent_session
   SET closed_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'), closed_reason = 'closed',
       updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
 WHERE kind = 'chat' AND closed_at IS NULL;
-- A closed chat row stays as history: it was a conversation. Its kind
-- becomes the nearest that remains.
UPDATE agent_session SET kind = 'terminal' WHERE kind = 'chat';

CREATE TABLE agent_session_new (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id         INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    kind              TEXT NOT NULL CHECK (kind IN ('terminal', 'action')),
    -- A harness registry key: no CHECK on purpose.
    harness           TEXT NOT NULL,
    title             TEXT NOT NULL DEFAULT '',
    resume_session_id TEXT NOT NULL DEFAULT '',
    -- The host it runs on; NULL is the local machine.
    host              TEXT,
    opened_at         TEXT NOT NULL,
    closed_at         TEXT,
    closed_reason     TEXT CHECK (closed_reason IN ('closed', 'thread_closed', 'stream_archived')),
    updated_at        TEXT NOT NULL,
    pid               INTEGER,
    last_seen_at      TEXT,
    stopped_by_user_at TEXT,
    native_title      TEXT NOT NULL DEFAULT '',
    fork_next         INTEGER NOT NULL DEFAULT 0 CHECK (fork_next IN (0, 1))
) STRICT;
INSERT INTO agent_session_new
    (id, thread_id, kind, harness, title, resume_session_id, host, opened_at, closed_at,
     closed_reason, updated_at, pid, last_seen_at, stopped_by_user_at, native_title, fork_next)
SELECT id, thread_id, kind, harness, title, resume_session_id, host, opened_at, closed_at,
       closed_reason, updated_at, pid, last_seen_at, stopped_by_user_at, native_title, fork_next
  FROM agent_session;
DROP TABLE agent_session;
ALTER TABLE agent_session_new RENAME TO agent_session;
CREATE INDEX idx_agent_session_thread ON agent_session(thread_id, opened_at DESC);
CREATE INDEX idx_agent_session_open ON agent_session(thread_id) WHERE closed_at IS NULL;
CREATE INDEX idx_agent_session_resume ON agent_session(resume_session_id)
    WHERE resume_session_id <> '';
