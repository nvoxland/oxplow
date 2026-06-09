-- Fix the FK target on task_effort.{start,end}_snapshot_id.
--
-- V1 declared these columns as `REFERENCES file_snapshot(id) ON
-- DELETE SET NULL`, but the snapshot-capture code stores the
-- `snapshot.id` (the V13 grouping row), not a `file_snapshot.id`.
-- The FK was lying about its target, so deletes on the wrong table
-- would have nullified these columns. Dormant until V2 of the
-- effort lifecycle wiring actually started populating them — at
-- which point the snapshot cleanup pass (`prune_older_than`) would
-- silently corrupt the references.
--
-- Rebuild the table with the correct FK. Existing values were
-- intended as snapshot.id all along, so they're preserved as-is.

CREATE TABLE task_effort_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER NOT NULL REFERENCES task(id) ON DELETE CASCADE,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    start_snapshot_id INTEGER REFERENCES snapshot(id) ON DELETE SET NULL,
    end_snapshot_id INTEGER REFERENCES snapshot(id) ON DELETE SET NULL,
    summary TEXT,
    impacts_json TEXT
);

INSERT INTO task_effort_new
    (id, task_id, thread_id, started_at, ended_at,
     start_snapshot_id, end_snapshot_id, summary, impacts_json)
SELECT id, task_id, thread_id, started_at, ended_at,
       start_snapshot_id, end_snapshot_id, summary, impacts_json
  FROM task_effort;

DROP TABLE task_effort;
ALTER TABLE task_effort_new RENAME TO task_effort;

CREATE INDEX idx_task_effort_task ON task_effort(task_id, started_at DESC);
CREATE INDEX idx_task_effort_thread ON task_effort(thread_id, started_at DESC);
