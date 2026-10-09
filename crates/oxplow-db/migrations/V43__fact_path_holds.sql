-- A per-path code gauge restates every function of each file it rescans,
-- and a rescan mostly repeats the file's last one: on this project 60% of
-- the facts were identical, and 99% identical but for their line (an edit
-- shifts the functions below it), ~800k facts a day.
--
-- A finished delta or full scan now holds a file's unchanged facts the
-- way V39's chains hold a complete scan's: `fact_path_hold` marks that a
-- capture holds a measure's facts for a path, stored at or after
-- `from_capture_id` (where that file was last stored whole) and not
-- closed before it. A fact that only moved stays one fact; `fact_line`
-- records its line from a capture on. `fact_store` says which facts a
-- capture holds, at which lines.
CREATE TABLE fact_path_hold (
    capture_id INTEGER NOT NULL REFERENCES metric_capture(id) ON DELETE CASCADE,
    measure_id INTEGER NOT NULL REFERENCES measure(id) ON DELETE CASCADE,
    path_id INTEGER NOT NULL REFERENCES fact_path(id),
    from_capture_id INTEGER NOT NULL,
    depth INTEGER NOT NULL,
    PRIMARY KEY (capture_id, measure_id, path_id)
) WITHOUT ROWID;
CREATE INDEX idx_fact_path_hold_path ON fact_path_hold(measure_id, path_id, capture_id);

CREATE TABLE fact_line (
    fact_id INTEGER NOT NULL REFERENCES fact(id) ON DELETE CASCADE,
    -- From this capture on (a bound, like `from_capture_id`).
    capture_id INTEGER NOT NULL,
    line INTEGER,
    PRIMARY KEY (fact_id, capture_id)
) WITHOUT ROWID;

-- A hold reads its file's facts by capture range.
DROP INDEX idx_fact_measure_path;
CREATE INDEX idx_fact_measure_path ON fact(measure_id, path_id, capture_id);
