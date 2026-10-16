-- Work handed from one thread to another (`oxplow.handoff.send`): who
-- sent it (a thread, and the agent session when an agent did), the open
-- thread it is for, the work item it hands over, its body, when the
-- recipient's agent was given it (`delivered_at`, stamped at a prompt),
-- and how it ended. A handoff is addressed to a thread, never to a
-- session: the thread outlives its processes.

CREATE TABLE handoff (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    from_thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    from_session_id INTEGER REFERENCES agent_session(id) ON DELETE SET NULL,
    from_actor TEXT NOT NULL CHECK (from_actor IN ('user', 'agent', 'effect', 'job', 'system')),
    to_thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    work_item TEXT,
    body TEXT NOT NULL,
    sent_at TEXT NOT NULL,
    delivered_at TEXT,
    delivered_to_session_id INTEGER REFERENCES agent_session(id) ON DELETE SET NULL,
    state TEXT NOT NULL DEFAULT 'open'
        CHECK (state IN ('open', 'taken', 'declined', 'withdrawn')),
    settled_at TEXT,
    settled_by_session_id INTEGER REFERENCES agent_session(id) ON DELETE SET NULL,
    note TEXT,
    CHECK (from_thread_id <> to_thread_id)
);
CREATE INDEX idx_handoff_to ON handoff(to_thread_id, state, sent_at);
CREATE INDEX idx_handoff_from ON handoff(from_thread_id, sent_at);
