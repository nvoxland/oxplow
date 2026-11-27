-- Tabs an agent opened for a device that was away (`.context/data-model.md`
-- "tab_queued"): the device takes them when its window next connects.
CREATE TABLE tab_queued (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    device_id TEXT NOT NULL,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    uri TEXT NOT NULL,
    queued_at TEXT NOT NULL
);
CREATE INDEX tab_queued_by_device ON tab_queued (device_id, id);
