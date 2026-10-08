-- A complete-scope capture restates its measure's whole population, and
-- consecutive scans of one worktree mostly repeat each other: a duplicate
-- scan restated ~19k blocks ~44 times a day with 0.3-7% changed. Such a
-- capture now stores only what changed since its producer's previous
-- finished capture in the same stream, and holds the facts it repeats
-- from there.
--
-- A fact stays held by the captures of its chain from its own capture up
-- to `last_capture_id`, the last one that held it (NULL while the newest
-- still does). `fact_chain` marks a capture as holding a measure's facts
-- that way, and bounds where they start (`from_capture_id`, the capture
-- the chain was last stored whole at; `depth` counts captures since).
-- `fact_store` says which facts a capture holds.
ALTER TABLE fact ADD COLUMN last_capture_id INTEGER;

CREATE TABLE fact_chain (
    capture_id INTEGER NOT NULL REFERENCES metric_capture(id) ON DELETE CASCADE,
    measure_id INTEGER NOT NULL REFERENCES measure(id) ON DELETE CASCADE,
    from_capture_id INTEGER NOT NULL,
    depth INTEGER NOT NULL,
    PRIMARY KEY (capture_id, measure_id)
) WITHOUT ROWID;
CREATE INDEX idx_fact_chain_measure ON fact_chain(measure_id, capture_id);
