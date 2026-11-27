-- A tab waits once per (device, thread, uri) (`.context/data-model.md`
-- "tab_queued"): the same page queued again changes nothing. The oldest row
-- of a repeat is kept.
CREATE TABLE tab_queued_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    device_id TEXT NOT NULL,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    uri TEXT NOT NULL,
    queued_at TEXT NOT NULL,
    UNIQUE (device_id, thread_id, uri)
);
INSERT OR IGNORE INTO tab_queued_new (id, device_id, thread_id, uri, queued_at)
    SELECT id, device_id, thread_id, uri, queued_at FROM tab_queued ORDER BY id;
DROP TABLE tab_queued;
ALTER TABLE tab_queued_new RENAME TO tab_queued;
CREATE INDEX tab_queued_by_device ON tab_queued (device_id, id);
