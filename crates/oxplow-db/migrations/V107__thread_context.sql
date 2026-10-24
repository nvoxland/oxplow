-- The thread's context basket: what the person collected for the thread's
-- agents while exploring — a page (`source_uri` its canonical ref) or a
-- piece of text (`text`, `source_uri` where it came from, when known).
-- The next prompt to any agent in the thread carries every undelivered
-- item and stamps it delivered. Every write is an `oxplow.context.*` command.
CREATE TABLE thread_context_item (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('page', 'text')),
    source_uri TEXT,
    label TEXT,
    text TEXT,
    added_at TEXT NOT NULL,
    delivered_at TEXT,
    delivered_session_id INTEGER,
    CHECK (kind = 'text' OR (source_uri IS NOT NULL AND text IS NULL)),
    CHECK (kind = 'page' OR text IS NOT NULL)
) STRICT;
CREATE INDEX thread_context_item_waiting
    ON thread_context_item (thread_id, delivered_at);
