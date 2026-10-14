-- The ops backfilled for snapshots taken before the op log existed carry
-- no cause; they read as manual takes, and `legacy` is no trigger any
-- more. SQLite can't change a CHECK in place, so the table is rebuilt.
UPDATE snapshot_op SET trigger = 'manual' WHERE trigger = 'legacy';
UPDATE event_log SET payload = json_set(payload, '$.trigger', 'manual')
 WHERE type = 'snapshot.taken' AND json_extract(payload, '$.trigger') = 'legacy';

CREATE TABLE snapshot_op_new (
    seq                INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id          INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    snapshot_id        INTEGER NOT NULL REFERENCES snapshot(id) ON DELETE CASCADE,
    parent_snapshot_id INTEGER REFERENCES snapshot(id) ON DELETE SET NULL,
    trigger            TEXT NOT NULL CHECK (trigger IN (
                          'turn_end', 'quiet', 'effort_start', 'effort_end', 'startup',
                          'manual', 'git_refs', 'head_moved', 'run_measured')),
    thread_id          INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    turn_id            INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL,
    effort_id          INTEGER REFERENCES effort(id) ON DELETE SET NULL,
    at                 TEXT NOT NULL,
    elapsed_ms         INTEGER NOT NULL,
    budget_ms          INTEGER,
    over_budget        INTEGER NOT NULL DEFAULT 0 CHECK (over_budget IN (0, 1)),
    file_count         INTEGER NOT NULL,
    provider           TEXT,
    contents           INTEGER NOT NULL DEFAULT 1,
    handle             TEXT
) STRICT;

INSERT INTO snapshot_op_new (seq, stream_id, snapshot_id, parent_snapshot_id, trigger,
                             thread_id, turn_id, effort_id, at, elapsed_ms, budget_ms,
                             over_budget, file_count, provider, contents, handle)
SELECT seq, stream_id, snapshot_id, parent_snapshot_id, trigger, thread_id, turn_id,
       effort_id, at, elapsed_ms, budget_ms, over_budget, file_count, provider, contents,
       handle
  FROM snapshot_op;

-- Keep AUTOINCREMENT's high-water mark.
CREATE TEMP TABLE snapshot_op_sequence AS
SELECT MAX(COALESCE((SELECT seq FROM sqlite_sequence WHERE name = 'snapshot_op'), 0),
           COALESCE((SELECT MAX(seq) FROM snapshot_op), 0)) AS seq;

PRAGMA legacy_alter_table = ON;
DROP TABLE snapshot_op;
ALTER TABLE snapshot_op_new RENAME TO snapshot_op;
PRAGMA legacy_alter_table = OFF;

DELETE FROM sqlite_sequence WHERE name = 'snapshot_op';
INSERT INTO sqlite_sequence (name, seq) SELECT 'snapshot_op', seq FROM snapshot_op_sequence;
DROP TABLE snapshot_op_sequence;

CREATE INDEX idx_snapshot_op_stream ON snapshot_op (stream_id, seq);
CREATE INDEX idx_snapshot_op_snapshot ON snapshot_op (snapshot_id);
CREATE INDEX idx_snapshot_op_turn ON snapshot_op (turn_id) WHERE turn_id IS NOT NULL;
CREATE INDEX idx_snapshot_op_effort ON snapshot_op (effort_id) WHERE effort_id IS NOT NULL;
