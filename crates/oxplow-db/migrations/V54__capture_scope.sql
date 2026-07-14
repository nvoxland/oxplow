-- tsk41 — capture completeness, the axis that makes codebase-wide metrics correct.
--
-- THE BUG. Tree gauges scan the LATEST SNAPSHOT, which after the initial full-tree
-- index is a per-commit DELTA (5-19 changed files). Their measures are
-- `semi-additive`, whose cross-time fold is "take the last capture". That fold is
-- only valid when EVERY capture restates the whole population — a delta capture
-- restates a SUBSET, so "last capture" reads as "the repo is only the 8 files I
-- just touched". `oxplow.rust.unsafe_blocks` read 0 while the repo had 15.
--
-- THE FIX. Completeness is a SEPARATE AXIS from additivity:
--   complete  — a capture restates the whole population (a coverage report, a
--               clippy run, a test run, the whole-tree duplication scan).
--   per-path  — a capture restates only THE PATHS IN ITS SNAPSHOT. The value is
--               folded as: for each (producer, path), the facts from the latest
--               capture of that producer whose snapshot contained that path.
--               (`SqliteFactStore::latest_tree_facts`, mirroring the existing
--               `SqliteSnapshotStore::tree_at` window fold.)
--
-- The scanned set comes from the capture's snapshot's `file_snapshot` rows — NOT
-- from the facts it emitted — so a file whose count drops to 0 (emits no fact) is
-- still superseded, a deleted file drops out via its `storage='deleted'` row, and
-- symbol-grained / multi-fact-per-path measures are superseded wholesale per file.
-- No zero-emission convention, no tombstone facts, no gauge-script changes.
--
-- WHY A NEW COLUMN, NOT A NEW `temporal_semantics` VALUE. `temporal_semantics`
-- carries a column-level CHECK (V43), and SQLite cannot ALTER a CHECK — it needs a
-- `measure` table rebuild, which fires `fact.measure_id ON DELETE CASCADE` under
-- the migration connection's `foreign_keys = ON` and WIPES EVERY FACT. That is the
-- exact trap V52 documents and refuses to go near. A plain ADD COLUMN has no such
-- hazard. Validation lives in oxplow-config + `CaptureScope::parse` instead — we
-- deliberately do NOT add another CHECK, which would just recreate the trap for the
-- next person. Tree measures stay `semi-additive`: "last wins", scoped per path.

ALTER TABLE measure ADD COLUMN capture_scope TEXT NOT NULL DEFAULT 'complete';

-- The snapshot-driven tree gauges. Everything else (coverage / test_case /
-- lint_hit / duplicate_lines / tokens / turn / nudge / cycle_time / task_effort /
-- effort_test_outcome) genuinely restates its whole population per capture and
-- stays `complete`.
UPDATE measure SET capture_scope = 'per-path' WHERE key IN (
    'oxplow.ast_hit',
    'oxplow.complexity',
    'oxplow.fn_length',
    'oxplow.parameter_count',
    'oxplow.todo'
);

-- Re-baseline. Every existing tree-gauge capture measured a delta under the old
-- (wrong) reading, so its facts are noise; a full-tree baseline scan repopulates
-- them. Scoped by PRODUCER (= the gauge key, `NewMetricCapture::done(stream,
-- gauge.key, ...)`) so we take that gauge's EMPTY captures too, and so
-- coverage/tests/tokens/lint/duplication captures are untouched. Facts CASCADE
-- away with their capture (`fact.capture_id → metric_capture(id) ON DELETE
-- CASCADE`) — deleting the children here also means the parent `measure` table is
-- never touched, so no cascade hazard.
DELETE FROM metric_capture WHERE producer IN (
    SELECT DISTINCT c.producer
      FROM metric_capture c
      JOIN fact f ON f.capture_id = c.id
      JOIN measure m ON m.id = f.measure_id
     WHERE m.capture_scope = 'per-path'
);

-- The fold joins fact→capture→file_snapshot; neither leg was indexed for it.
CREATE INDEX IF NOT EXISTS idx_fact_measure_path ON fact(measure_id, path);
CREATE INDEX IF NOT EXISTS idx_metric_capture_snapshot ON metric_capture(snapshot_id);
