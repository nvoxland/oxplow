-- A change may be any two revisions: kind `range`, target `<base>..<head>`
-- (each side a revision as `rev` is spelled: `working`, `snap:<id>`,
-- `git:<sha>`; the base empty for "from nothing"). Rebuild the table to
-- widen its kind CHECK, as SQLite cannot alter one in place, keeping every
-- row: the analysis tables reference `change(id)` and ids are preserved.
-- Migrations run with foreign keys off, so dropping the parent leaves the
-- children's rows in place.

-- Preserve AUTOINCREMENT's high-water mark, including ids above MAX(id).
CREATE TEMP TABLE change_sequence AS
SELECT MAX(COALESCE((SELECT seq FROM sqlite_sequence WHERE name = 'change'), 0),
           COALESCE((SELECT MAX(id) FROM change), 0)) AS seq;

CREATE TABLE change_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL,
    -- `commit` | `effort` | `working` | `turn` | `range`
    kind TEXT NOT NULL CHECK (kind IN ('commit', 'effort', 'working', 'turn', 'range')),
    -- The sha, the effort id, the turn id, `<base>..<head>` for a range,
    -- or '' for the working tree.
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

INSERT INTO change_new (
    id, stream_id, kind, target, base_revision, head_revision, status, error,
    computed_at, snapshot_id, events_to, files_at, files_events_to, conflicted,
    in_progress, elapsed_ms, analyzed_from, duplicates_events_to
)
SELECT id, stream_id, kind, target, base_revision, head_revision, status, error,
       computed_at, snapshot_id, events_to, files_at, files_events_to, conflicted,
       in_progress, elapsed_ms, analyzed_from, duplicates_events_to
  FROM change;

-- Views are removed before migrations, but legacy rename mode also avoids
-- rewriting any dependent SQL while the replacement table is installed.
PRAGMA legacy_alter_table = ON;
DROP TABLE change;
ALTER TABLE change_new RENAME TO change;
PRAGMA legacy_alter_table = OFF;

DELETE FROM sqlite_sequence WHERE name = 'change';
INSERT INTO sqlite_sequence (name, seq)
SELECT 'change', seq FROM change_sequence;

DROP TABLE change_sequence;
