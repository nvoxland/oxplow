-- The left nav's layout is each device's own (`.context/data-model.md`
-- "panel_layout"): the same three lists (the project's, each stream's, each
-- thread's), kept per device. The rows there were go: every device starts
-- from the panels' declared order.
CREATE TABLE panel_layout_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    device_id TEXT NOT NULL,
    panel TEXT NOT NULL,
    visibility_scope TEXT NOT NULL CHECK (visibility_scope IN ('thread', 'stream', 'project')),
    thread_id INTEGER REFERENCES threads(id) ON DELETE CASCADE,
    stream_id INTEGER REFERENCES streams(id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    hidden INTEGER NOT NULL DEFAULT 0 CHECK (hidden IN (0, 1)),
    collapsed INTEGER NOT NULL DEFAULT 0 CHECK (collapsed IN (0, 1)),
    CHECK ((visibility_scope = 'thread') = (thread_id IS NOT NULL)),
    CHECK ((visibility_scope = 'stream') = (stream_id IS NOT NULL)),
    CHECK (visibility_scope = 'project' OR hidden = 0)
) STRICT;
DROP TABLE panel_layout;
ALTER TABLE panel_layout_new RENAME TO panel_layout;
CREATE UNIQUE INDEX panel_layout_owner
    ON panel_layout (device_id, panel, visibility_scope, coalesce(thread_id, 0), coalesce(stream_id, 0));
