-- P2.10 (tsk434) — a turn is a change-analysis target: `change.kind`
-- gains `turn` (target = the agent_turn id), analyzed from the turn's
-- start snapshot to its end snapshot — "what changed this turn".
--
-- SQLite can't widen a CHECK in place, so the table is rebuilt. `change`
-- is a derived cache (`ensure_change` recomputes any row on demand), so
-- it's emptied first: that cascades its analysis children
-- (change_file, change_function, …) cleanly before the swap, and nothing
-- the user made is lost. The children's FKs name `change`, which the
-- renamed table takes over.

DELETE FROM change;
DROP VIEW v_change;

CREATE TABLE change_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL,
    -- `commit` | `effort` | `working` | `turn`
    kind TEXT NOT NULL CHECK (kind IN ('commit', 'effort', 'working', 'turn')),
    -- The sha, the effort id, the turn id, or '' for the working tree.
    target TEXT NOT NULL,
    base_label TEXT,
    head_label TEXT,
    status TEXT NOT NULL CHECK (status IN ('pending', 'running', 'done', 'failed')),
    error TEXT,
    computed_at TEXT,
    UNIQUE (stream_id, kind, target)
);
DROP TABLE change;
ALTER TABLE change_new RENAME TO change;

CREATE VIEW v_change AS
SELECT id, stream_id, kind, target, base_label, head_label, status, error, computed_at
FROM change;
