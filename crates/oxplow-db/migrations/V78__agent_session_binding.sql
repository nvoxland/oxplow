-- Which native conversation an agent slot has been bound to, and when
-- (.context/data-model.md "agent_session"): the slot's resume id is its
-- open binding's; a native `/resume`, a fork, the person choosing one,
-- or a fresh start ends one and opens the next. `native_title` is the
-- name the harness gives the bound conversation (cleared on a switch,
-- never copied across); `fork_next` asks the next launch to fork the
-- bound conversation rather than resume it.
ALTER TABLE agent_session ADD COLUMN native_title TEXT NOT NULL DEFAULT '';
ALTER TABLE agent_session ADD COLUMN fork_next INTEGER NOT NULL DEFAULT 0 CHECK (fork_next IN (0, 1));

CREATE TABLE agent_session_binding (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_session_id INTEGER NOT NULL REFERENCES agent_session(id) ON DELETE CASCADE,
    native_id        TEXT NOT NULL,
    origin           TEXT NOT NULL CHECK (origin IN ('started', 'switched', 'forked', 'chosen')),
    bound_at         TEXT NOT NULL,
    unbound_at       TEXT
) STRICT;
CREATE INDEX idx_agent_session_binding_session
    ON agent_session_binding(agent_session_id, bound_at);
CREATE UNIQUE INDEX idx_agent_session_binding_open
    ON agent_session_binding(agent_session_id) WHERE unbound_at IS NULL;

-- Every slot bound now was bound when it opened, as far as anyone knows.
INSERT INTO agent_session_binding (agent_session_id, native_id, origin, bound_at)
SELECT id, resume_session_id, 'started', opened_at
  FROM agent_session WHERE resume_session_id <> '';
