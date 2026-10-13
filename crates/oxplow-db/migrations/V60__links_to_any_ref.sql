-- A work item's link may point at any ref, not only another item
-- (`.context/refs.md` "Using refs"): `about` says what the item is about
-- (`collector:triage/hot`), and a link may dangle — what it names can be
-- deleted, or its extension removed. oxplow's tasks keep the target as its
-- ref, and `work_item_link` records the target's kind beside it.

CREATE TABLE task_link_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    from_item_id INTEGER NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    to_ref TEXT NOT NULL,
    link_type TEXT NOT NULL CHECK (link_type IN ('blocks', 'relates_to', 'discovered_from',
        'duplicates', 'supersedes', 'replies_to', 'about')),
    created_at TEXT NOT NULL,
    UNIQUE (from_item_id, to_ref, link_type)
);
INSERT OR IGNORE INTO task_link_new (id, thread_id, from_item_id, to_ref, link_type, created_at)
SELECT id, thread_id, from_item_id, 'work_item:oxplow:tsk' || to_item_id, link_type, created_at
FROM task_link;
DROP TABLE task_link;
ALTER TABLE task_link_new RENAME TO task_link;
CREATE INDEX idx_task_link_thread_from ON task_link (thread_id, from_item_id);
CREATE INDEX idx_task_link_to ON task_link (to_ref);

ALTER TABLE work_item_link ADD COLUMN to_kind TEXT NOT NULL DEFAULT 'work_item';
