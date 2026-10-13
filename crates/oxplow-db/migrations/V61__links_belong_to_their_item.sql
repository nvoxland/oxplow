-- A work item's link belongs to the item, not to a thread: nothing reads
-- the thread it was made in, and a link made by an effect with no thread
-- (a repair item `about` its contribution) has none.

CREATE TABLE task_link_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    from_item_id INTEGER NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    to_ref TEXT NOT NULL,
    link_type TEXT NOT NULL CHECK (link_type IN ('blocks', 'relates_to', 'discovered_from',
        'duplicates', 'supersedes', 'replies_to', 'about')),
    created_at TEXT NOT NULL,
    UNIQUE (from_item_id, to_ref, link_type)
);
INSERT INTO task_link_new (id, from_item_id, to_ref, link_type, created_at)
SELECT id, from_item_id, to_ref, link_type, created_at FROM task_link;
DROP TABLE task_link;
ALTER TABLE task_link_new RENAME TO task_link;
CREATE INDEX idx_task_link_from ON task_link (from_item_id);
CREATE INDEX idx_task_link_to ON task_link (to_ref);
