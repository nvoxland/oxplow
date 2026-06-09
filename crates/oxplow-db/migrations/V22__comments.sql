-- Comments: threaded annotations anchored to a text selection on any
-- page (wiki body, code file lines, task detail, …). Integer PK ids
-- (no UUIDs). See .context/data-model.md.

-- The thread anchor + metadata. stream_id is the hard scope so the
-- agent can list comments per stream; thread_id is the origin agent
-- thread and is nullable (ON DELETE SET NULL) so content comments
-- survive the thread being archived/deleted.
CREATE TABLE comment (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    thread_id INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    target_kind TEXT NOT NULL,
    target_id TEXT NOT NULL,
    -- The selected text: the durable anchor + the context handed to
    -- the agent.
    quote TEXT NOT NULL,
    -- Opaque per-surface position hint (re-validated on load).
    anchor_json TEXT NOT NULL,
    -- 'note' (note-to-self) | 'followup' (wants the agent to act).
    intent TEXT NOT NULL DEFAULT 'note',
    -- 'open' | 'resolved'.
    status TEXT NOT NULL DEFAULT 'open',
    -- 1 when the quote could no longer be located in current content;
    -- still listed in the inbox, just without an inline highlight.
    orphaned INTEGER NOT NULL DEFAULT 0,
    author TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    -- Bumped on every new message; drives sorting + GC.
    last_activity_at TEXT NOT NULL
);
CREATE INDEX idx_comment_stream ON comment(stream_id, status, last_activity_at DESC);
CREATE INDEX idx_comment_thread ON comment(thread_id, last_activity_at DESC);
CREATE INDEX idx_comment_target ON comment(target_kind, target_id);

-- Every message in a thread, including the first one.
CREATE TABLE comment_message (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    comment_id INTEGER NOT NULL REFERENCES comment(id) ON DELETE CASCADE,
    -- Free-form, e.g. 'user' or 'agent'.
    author TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_comment_message_comment ON comment_message(comment_id, created_at);
