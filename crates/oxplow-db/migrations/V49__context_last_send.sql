-- A thread's last send, kept so the person can put it back. A send copies
-- what it carried here (the thread's earlier copy replaced: one send per
-- thread, never a history) and deletes the items; Put Back returns these
-- to the basket they came from — by name, made again if it was deleted —
-- and sets Send Next again when that basket was the one sent.
CREATE TABLE context_sent_item (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    basket_name TEXT,
    stream_id INTEGER REFERENCES streams(id) ON DELETE SET NULL,
    kind TEXT NOT NULL CHECK (kind IN ('page', 'text')),
    source_uri TEXT,
    label TEXT,
    text TEXT,
    around TEXT,
    note TEXT,
    added_at TEXT NOT NULL,
    sent_at TEXT NOT NULL
) STRICT;

CREATE INDEX context_sent_item_thread ON context_sent_item (thread_id);
