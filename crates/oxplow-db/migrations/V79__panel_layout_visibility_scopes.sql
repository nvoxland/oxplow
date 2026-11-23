-- The left nav's layout is three lists: the project's, each stream's and
-- each thread's (`visibility_scope`, as bookmarks). A viewer sees its
-- thread's panels, then its stream's, then the project's. The project
-- list names every panel the person has placed: one placed in a thread or
-- stream keeps a hidden project row, so it isn't new elsewhere. Only a
-- project row can be hidden. The rows there were are the project's.
CREATE TABLE panel_layout_scoped (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
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
INSERT INTO panel_layout_scoped (panel, visibility_scope, position, hidden, collapsed)
SELECT panel, 'project', position, hidden, collapsed FROM panel_layout ORDER BY position;
DROP TABLE panel_layout;
ALTER TABLE panel_layout_scoped RENAME TO panel_layout;
CREATE UNIQUE INDEX panel_layout_owner
    ON panel_layout (panel, visibility_scope, coalesce(thread_id, 0), coalesce(stream_id, 0));
