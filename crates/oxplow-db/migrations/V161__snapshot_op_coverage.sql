-- tsk883: a run's coverage is pinned to a snapshot taken when it is
-- recorded (`coverage`), the code the run measured. The CHECK can't
-- change in place, so `snapshot_op` is rebuilt.
CREATE TABLE snapshot_op_new (
    seq                INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id          INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    snapshot_id        INTEGER NOT NULL REFERENCES snapshot(id) ON DELETE CASCADE,
    parent_snapshot_id INTEGER REFERENCES snapshot(id) ON DELETE SET NULL,
    trigger            TEXT NOT NULL CHECK (trigger IN (
                          'turn_end', 'quiet', 'effort_start', 'effort_end', 'startup',
                          'manual', 'git_refs', 'head_moved', 'coverage', 'legacy')),
    thread_id          INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    turn_id            INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL,
    effort_id          INTEGER REFERENCES effort(id) ON DELETE SET NULL,
    at                 TEXT NOT NULL,
    elapsed_ms         INTEGER NOT NULL,
    budget_ms          INTEGER,
    over_budget        INTEGER NOT NULL DEFAULT 0 CHECK (over_budget IN (0, 1)),
    file_count         INTEGER NOT NULL
) STRICT;
INSERT INTO snapshot_op_new
    (seq, stream_id, snapshot_id, parent_snapshot_id, trigger, thread_id, turn_id, effort_id,
     at, elapsed_ms, budget_ms, over_budget, file_count)
SELECT seq, stream_id, snapshot_id, parent_snapshot_id, trigger, thread_id, turn_id, effort_id,
       at, elapsed_ms, budget_ms, over_budget, file_count
FROM snapshot_op;
DROP TABLE snapshot_op;
ALTER TABLE snapshot_op_new RENAME TO snapshot_op;
CREATE INDEX idx_snapshot_op_stream ON snapshot_op (stream_id, seq);
CREATE INDEX idx_snapshot_op_snapshot ON snapshot_op (snapshot_id);
CREATE INDEX idx_snapshot_op_turn ON snapshot_op (turn_id) WHERE turn_id IS NOT NULL;
CREATE INDEX idx_snapshot_op_effort ON snapshot_op (effort_id) WHERE effort_id IS NOT NULL;
