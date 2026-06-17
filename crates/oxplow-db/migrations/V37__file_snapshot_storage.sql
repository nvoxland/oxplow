-- Make the per-file storage class an explicit `storage` column on
-- `file_snapshot`, replacing the implicit 2-bit `(blob_hash NULL?,
-- oversize?)` encoding.
--
-- This is the schema half of "git-sourced snapshot baselines": a
-- captured file whose working-tree bytes are byte-identical to a
-- committed blob no longer copies its content into oxplow's blob
-- store. Instead the row records `storage = 'git'` and stuffs the
-- **git blob OID** into `blob_hash`, so the bytes are recovered on
-- demand via `git cat-file` (libgit2 `find_blob`) instead of from
-- `.oxplow/snapshots/objects/`. A clean checkout of a large repo
-- therefore boots without re-blobbing every tracked file.
--
-- `storage` values (one per "where do the bytes live"):
--   * 'oxplow'   — blob_hash is an xxh3-128, bytes in the blob store
--   * 'git'      — blob_hash is a git blob OID, bytes in the git odb
--   * 'oversize' — blob_hash NULL; file too big to hash, size+mtime tracked
--   * 'deleted'  — blob_hash NULL; tombstone row marking a path gone
--
-- The legacy encoding maps exactly:
--   oversize = 1                       -> 'oversize'
--   oversize = 0 AND blob_hash IS NULL -> 'deleted'   (tombstone)
--   otherwise                          -> 'oxplow'
-- (No pre-V37 row can be 'git' — that class didn't exist.)

CREATE TABLE file_snapshot_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    blob_hash TEXT,
    size_bytes INTEGER NOT NULL DEFAULT 0,
    captured_at TEXT NOT NULL,
    storage TEXT NOT NULL DEFAULT 'oxplow'
        CHECK (storage IN ('oxplow', 'git', 'oversize', 'deleted')),
    snapshot_id INTEGER REFERENCES snapshot(id) ON DELETE CASCADE,
    mtime_ms INTEGER
);

INSERT INTO file_snapshot_new
    (id, stream_id, path, blob_hash, size_bytes, captured_at, storage, snapshot_id, mtime_ms)
SELECT id, stream_id, path, blob_hash, size_bytes, captured_at,
       CASE
           WHEN oversize = 1 THEN 'oversize'
           WHEN blob_hash IS NULL THEN 'deleted'
           ELSE 'oxplow'
       END,
       snapshot_id, mtime_ms
FROM file_snapshot;

DROP TABLE file_snapshot;
ALTER TABLE file_snapshot_new RENAME TO file_snapshot;

CREATE INDEX idx_file_snapshot_stream_path ON file_snapshot(stream_id, path, captured_at DESC);
CREATE INDEX idx_file_snapshot_path ON file_snapshot(path, captured_at DESC);
CREATE INDEX idx_file_snapshot_snapshot ON file_snapshot(snapshot_id);
