-- Make `snapshot.stream_id` and `file_snapshot.stream_id` NOT NULL.
--
-- Snapshot capture must always be stream-aware: every captured file
-- belongs to a specific stream's worktree, and the Local history
-- panel queries `WHERE stream_id = ?`. Pre-V16 rows were written
-- with NULL stream_ids (the singleton SnapshotCaptureService was
-- wired with `stream_id = None`), so they never showed up.
--
-- Existing data is cleared as part of this migration — the rows were
-- effectively orphan history that nothing could query anyway.

-- Wipe existing rows. `task_effort.{start,end}_snapshot_id` FKs use
-- ON DELETE SET NULL, so the deletes cascade-null cleanly.
DELETE FROM file_snapshot;
DELETE FROM snapshot;

-- SQLite doesn't support `ALTER COLUMN ... NOT NULL`, so we rebuild
-- both tables.
CREATE TABLE file_snapshot_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    blob_hash TEXT,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    captured_at TEXT NOT NULL,
    oversize INTEGER NOT NULL DEFAULT 0,
    snapshot_id INTEGER REFERENCES snapshot(id) ON DELETE CASCADE,
    mtime_ms INTEGER
);
DROP TABLE file_snapshot;
ALTER TABLE file_snapshot_new RENAME TO file_snapshot;
CREATE INDEX idx_file_snapshot_stream_path ON file_snapshot(stream_id, path, captured_at DESC);
CREATE INDEX idx_file_snapshot_path ON file_snapshot(path, captured_at DESC);
CREATE INDEX idx_file_snapshot_snapshot ON file_snapshot(snapshot_id);

CREATE TABLE snapshot_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    git_commit TEXT
);
DROP TABLE snapshot;
ALTER TABLE snapshot_new RENAME TO snapshot;
CREATE INDEX idx_snapshot_stream ON snapshot(stream_id, created_at DESC);
