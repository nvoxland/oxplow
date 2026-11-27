-- The stream a window shows and the thread it shows in each stream are each
-- device's own (`.context/data-model.md` "device_view_state"). The shared
-- rows go: every device starts from the defaults (the primary stream, a
-- stream's first open thread).
DROP TABLE runtime_state;
CREATE TABLE device_view_state (
    device_id TEXT PRIMARY KEY,
    current_stream_id INTEGER REFERENCES streams(id) ON DELETE SET NULL
);
DROP TABLE thread_selection;
CREATE TABLE thread_selection (
    device_id TEXT NOT NULL,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    selected_thread_id INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    PRIMARY KEY (device_id, stream_id)
);
