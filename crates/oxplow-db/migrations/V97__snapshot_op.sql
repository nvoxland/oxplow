-- P2.2 (tsk424) — the snapshot operation log (.context/target-architecture.md
-- §6.1; .context/data-model.md "snapshot_op").
--
-- Every take records one op: which snapshot the worktree is at after it,
-- its parent (the snapshot it was at before), why it ran, what it is
-- anchored to (thread / turn / effort), how long it took against its
-- budget, and how many rows it wrote. A take that found nothing new still
-- records an op pointing at the unchanged snapshot, so "what did this
-- turn change" is answerable (parent → snapshot) even when the answer is
-- "nothing". The op row, the snapshot row, its file rows and the
-- `snapshot.taken` event are written in ONE transaction
-- (`SqliteSnapshotStore::record_take`).
--
-- Existing snapshots get one `legacy` op each, parent = the previous
-- snapshot of the same stream by (created_at, id), so ancestry reads the
-- same way for old and new history.

CREATE TABLE snapshot_op (
    seq                INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id          INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    snapshot_id        INTEGER NOT NULL REFERENCES snapshot(id) ON DELETE CASCADE,
    parent_snapshot_id INTEGER REFERENCES snapshot(id) ON DELETE SET NULL,
    trigger            TEXT NOT NULL CHECK (trigger IN (
                          'turn_end', 'quiet', 'effort_start', 'effort_end', 'startup',
                          'manual', 'git_refs', 'head_moved', 'legacy')),
    thread_id          INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    turn_id            INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL,
    effort_id          INTEGER REFERENCES task_effort(id) ON DELETE SET NULL,
    at                 TEXT NOT NULL,
    elapsed_ms         INTEGER NOT NULL,
    budget_ms          INTEGER,
    over_budget        INTEGER NOT NULL DEFAULT 0 CHECK (over_budget IN (0, 1)),
    file_count         INTEGER NOT NULL
) STRICT;

CREATE INDEX idx_snapshot_op_stream ON snapshot_op (stream_id, seq);
CREATE INDEX idx_snapshot_op_snapshot ON snapshot_op (snapshot_id);
CREATE INDEX idx_snapshot_op_turn ON snapshot_op (turn_id) WHERE turn_id IS NOT NULL;
CREATE INDEX idx_snapshot_op_effort ON snapshot_op (effort_id) WHERE effort_id IS NOT NULL;

INSERT INTO snapshot_op (stream_id, snapshot_id, parent_snapshot_id, trigger, at, elapsed_ms, file_count)
SELECT s.stream_id,
       s.id,
       (SELECT p.id FROM snapshot p
         WHERE p.stream_id = s.stream_id
           AND (p.created_at < s.created_at OR (p.created_at = s.created_at AND p.id < s.id))
         ORDER BY p.created_at DESC, p.id DESC
         LIMIT 1),
       'legacy',
       s.created_at,
       0,
       (SELECT count(*) FROM file_snapshot f WHERE f.snapshot_id = s.id)
  FROM snapshot s
 ORDER BY s.created_at, s.id;

CREATE VIEW v_snapshot_op AS
SELECT seq, stream_id, snapshot_id, parent_snapshot_id, trigger, thread_id, turn_id,
       effort_id, at, elapsed_ms, budget_ms, over_budget, file_count
  FROM snapshot_op;
