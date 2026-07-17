-- tsk103 (review of the metric cube work) — three structural fixes.
--
-- 1. THE EPOCH FENCE. The cube build runs outside any transaction with the
--    prune (by design, see V62), and the prune's invalidation can land MID
--    build: the builder computed its todo-list from the pre-wipe watermark,
--    keeps folding, and its next write re-plants a watermark that covers
--    captures whose rows the wipe deleted — "covered but rowless", served as
--    explicit 0s. The fence is one global counter: every invalidation bumps
--    it, and `write_cube_rows` refuses to commit when the epoch moved since
--    the builder read it — the stale pass abandons, and the next build folds
--    from the post-wipe (empty) watermark. Coarse on purpose: invalidations
--    are rare (prune-with-drops, grain/scope changes), and a false abandon
--    costs one re-fold, never a wrong number.
CREATE TABLE metric_cube_epoch (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    epoch INTEGER NOT NULL
);
INSERT INTO metric_cube_epoch (id, epoch) VALUES (1, 0);

-- 2. Index the `metric_live_fact.fact_id` FK. SQLite enforces an unindexed
--    child FK by scanning the child table once per deleted parent row, so the
--    prune's fact CASCADE was O(deleted facts × live rows) — V62 indexed
--    `metric_cube(capture_id)` for exactly this reason and this one was
--    missed.
CREATE INDEX idx_metric_live_fact_fact ON metric_live_fact(fact_id);

-- 3. `metric_cube_state` rows now cascade with their stream. Every other cube
--    row already died with the stream (captures/facts cascade), but the
--    watermark rows lingered — inert until a stream id is reused, where a
--    stale row is both a false watermark and a false "branch seeded" marker
--    (an inheriting branch would skip its seed). Recreated because SQLite
--    can't add an FK in place; the cube is disposable (V62).
DROP TABLE metric_cube_state;
CREATE TABLE metric_cube_state (
    measure_id INTEGER NOT NULL REFERENCES measure(id) ON DELETE CASCADE,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    branch TEXT NOT NULL,
    last_capture_id INTEGER NOT NULL,
    last_captured_at TEXT NOT NULL,
    PRIMARY KEY (measure_id, stream_id, branch)
) WITHOUT ROWID;

-- Clear the rest of the cube alongside the dropped watermarks: this review
-- also changed the fold's INPUTS (non-`done` captures no longer enter
-- `captures_for_producers`), and anything that changes what a replay would
-- compute must invalidate (the tsk100 rule, generalized).
DELETE FROM metric_cube;
DELETE FROM metric_live_fact;
