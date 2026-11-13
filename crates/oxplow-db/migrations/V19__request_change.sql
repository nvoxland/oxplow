-- A request is a change kind: its turns' span on one stream's snapshot
-- line, limited to the files its turns changed (keyed by request id, one
-- row per stream). SQLite can't alter a CHECK, so the table is rebuilt
-- (migrations run with foreign keys off; the rows keep their ids, so
-- change_file and the rest still point at them).
CREATE TABLE change_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL,
    -- `commit` | `effort` | `working` | `turn` | `range` | `request`
    kind TEXT NOT NULL CHECK (kind IN ('commit', 'effort', 'working', 'turn', 'range', 'request')),
    -- The sha, the effort id, the turn id, the request id,
    -- `<base>..<head>` for a range, or '' for the working tree.
    target TEXT NOT NULL,
    base_revision TEXT,
    head_revision TEXT,
    status TEXT NOT NULL CHECK (status IN ('pending', 'running', 'done', 'failed')),
    error TEXT,
    computed_at TEXT,
    snapshot_id INTEGER,
    events_to INTEGER,
    files_at TEXT,
    files_events_to INTEGER,
    conflicted INTEGER,
    in_progress TEXT,
    elapsed_ms INTEGER,
    analyzed_from TEXT,
    duplicates_events_to INTEGER,
    UNIQUE (stream_id, kind, target)
);
INSERT INTO change_new SELECT id, stream_id, kind, target, base_revision, head_revision, status,
       error, computed_at, snapshot_id, events_to, files_at, files_events_to, conflicted,
       in_progress, elapsed_ms, analyzed_from, duplicates_events_to
  FROM "change";
DROP TABLE "change";
ALTER TABLE change_new RENAME TO "change";
