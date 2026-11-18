-- Named context baskets. A context item is in a thread's own basket
-- (`thread_id`) or in a named basket (`basket_id`) the person collects into
-- across every stream, and attaches to a thread's next prompt. A delivered
-- item is deleted: the turn's `prompt_mention` links and the transcript are
-- the record of what the agent was given, so the delivery columns go. Each
-- item names the stream it was collected in, so one delivered into another
-- stream is read there. The table is rebuilt (`thread_id` becomes nullable)
-- under its new name; what still waits is kept.
CREATE TABLE context_basket (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE,
    attached_thread_id INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    created_at TEXT NOT NULL
) STRICT;

CREATE TABLE context_item (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER REFERENCES threads(id) ON DELETE CASCADE,
    basket_id INTEGER REFERENCES context_basket(id) ON DELETE CASCADE,
    stream_id INTEGER REFERENCES streams(id) ON DELETE SET NULL,
    kind TEXT NOT NULL CHECK (kind IN ('page', 'text')),
    source_uri TEXT,
    label TEXT,
    text TEXT,
    around TEXT,
    note TEXT,
    added_at TEXT NOT NULL,
    CHECK ((thread_id IS NULL) <> (basket_id IS NULL)),
    CHECK (kind = 'text' OR (source_uri IS NOT NULL AND text IS NULL)),
    CHECK (kind = 'page' OR text IS NOT NULL)
) STRICT;

INSERT INTO context_item (id, thread_id, stream_id, kind, source_uri, label, text, around, note, added_at)
  SELECT c.id, c.thread_id, t.stream_id, c.kind, c.source_uri, c.label, c.text, c.around, c.note, c.added_at
    FROM thread_context_item c
    JOIN threads t ON t.id = c.thread_id
   WHERE c.delivered_at IS NULL;

DROP TABLE thread_context_item;
DELETE FROM table_generation WHERE name = 'thread_context_item';

CREATE INDEX context_item_thread ON context_item (thread_id);
CREATE INDEX context_item_basket ON context_item (basket_id);
CREATE INDEX context_basket_attached ON context_basket (attached_thread_id);
