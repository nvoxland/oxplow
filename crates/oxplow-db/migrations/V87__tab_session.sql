-- Each device's tabs of each thread (`.context/data-model.md` "tab_session"):
-- the window's session of tabs, kept by the daemon so a device's tabs are
-- its own and a project's database holds only its own threads' tabs.
-- `revision` is bumped by each save; a save gives the one it read.
CREATE TABLE tab_session (
    device_id TEXT NOT NULL,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    session_json TEXT NOT NULL,
    revision INTEGER NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (device_id, thread_id)
);
