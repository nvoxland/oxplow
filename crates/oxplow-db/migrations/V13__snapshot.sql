-- Introduce a lightweight `snapshot` row that groups the
-- `file_snapshot` entries produced by one `request_snapshot()` call.
-- This gives callers (task efforts, etc.) a single rowid version
-- handle without bringing back the old manifest/version_hash model.
--
-- Design constraints (see thread b-019e183a-63c3):
--  * No `source` column — a snapshot stands alone.
--  * No `effort_id` — task→snapshot mapping lives on the task/effort
--    tables (`task_effort.start_snapshot_id` / `end_snapshot_id`),
--    so the relationship is unidirectional.
--  * `request_snapshot()` only inserts a snapshot row when the dirty
--    set is non-empty; otherwise it returns the most-recent existing
--    snapshot id so consecutive no-op requests don't pollute the
--    table.

CREATE TABLE snapshot (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER REFERENCES streams(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_snapshot_stream ON snapshot(stream_id, created_at DESC);

-- file_snapshot rows now point at their grouping snapshot. NULL is
-- allowed for legacy rows captured before this migration (the
-- startup sweep and any pre-V13 captures); new rows always populate
-- it.
ALTER TABLE file_snapshot ADD COLUMN snapshot_id INTEGER REFERENCES snapshot(id) ON DELETE CASCADE;
CREATE INDEX idx_file_snapshot_snapshot ON file_snapshot(snapshot_id);
