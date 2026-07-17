-- tsk96 — the metric aggregate cube: materialize the fold, stop re-folding it
-- on every read.
--
-- THE PROBLEM. A metric is a spec evaluated over facts at READ time. For a
-- PARTIAL-scope measure (`per-path` / `per-subject`) that read is a stateful
-- replay: point[N] = agg(state[N]), state[N] = state[N-1] - restated(N) +
-- facts(N). So one sparkline over `oxplow.test_case` decodes all 240k facts and
-- replays 125 captures to emit 125 points. `RecordedMetricsPage` asks per spec,
-- 5 specs share those 2 measures, and it refreshes every ~10s while an agent
-- works => ~1M fact decodes (a dims_json alloc + serde parse each) to produce
-- ~600 numbers. That is the CPU burn.
--
-- THE FIX. Store the fold's OUTPUT. `metric_cube` holds one row per
-- (measure, capture, promoted dims) carrying the DECOMPOSABLE components
-- (count/sum/min/max/numerator/denominator), so a read is a GROUP BY over ~152
-- rows instead of a replay over 240k facts. You cannot GROUP BY a fold; you CAN
-- GROUP BY a pre-folded cube — materialization is what unlocks the SQL path.
--
-- WHY THESE COLUMNS. Every aggregation in the catalog (sum 40, avg 10, count 9,
-- max 6, ratio 3) is decomposable: bucket the facts, aggregate each bucket, merge
-- the buckets, and you land exactly where aggregating all of them at once lands.
-- `metric_engine::Cell::project` and `aggregate_facts` are the two sides of that
-- identity and are pinned equal by test. `last` is NOT decomposable (merging
-- destroys the ordering it means) and refuses, falling back to the facts.
--
-- Ratio components are stored as SEPARATE sums so a read re-derives Sn/Sd —
-- never a mean of percentages. They accumulate only from facts carrying BOTH
-- components, matching `aggregate_facts`.
--
-- THE GRAIN IS (measure, capture, promoted dims) AND THE FLOOR IS THE CAPTURE.
-- The cube drops ONLY the subject axis ("which of the 2,496 tests") and never
-- the time axis. A capture IS one scan/run, so everything hanging off it —
-- snapshot_id, effort_id, thread_id, branch, closest_git_version, stream_id —
-- stays reachable through the JOIN, and within-effort/per-snapshot metric deltas
-- keep working. Never aggregate coarser than a capture (per-day, per-commit):
-- that would destroy snapshot resolution. Branch/thread/stream stay DIMENSIONS
-- you can group by, never partitions that hide rows.
--
-- Deliberately lean: no denormalized capture attributes. They come from the JOIN
-- to `metric_capture` (152 rows — trivial), which is exactly the star schema
-- `fact` already uses: an aggregate fact table beside the transaction fact table,
-- sharing the same capture dimension.
--
-- THE CUBE NEVER REPLACES THE FACTS. It is a lossy projection by construction —
-- that lossiness IS the speedup. Reads needing the subject axis back stay on the
-- facts, permanently and by design: a `min_value >= 11` threshold spec
-- (`oxplow.high_complexity_fns`, `oxplow.long_functions`) whose values the cube
-- summed away; the findings/drill-in ("which test, which file, which line");
-- `group_by` on an unpromoted dim (`subject` would need a bucket per test — i.e.
-- the fact table again). This is ordinary aggregate navigation: the query layer
-- picks the smallest table that can answer, and the base facts answer everything.
-- Consequently the cube is 100% derivable and DISPOSABLE: delete every row and
-- the only thing lost is speed. Never let a read depend on it for data.

CREATE TABLE metric_cube (
    measure_id INTEGER NOT NULL REFERENCES measure(id) ON DELETE CASCADE,
    capture_id INTEGER NOT NULL REFERENCES metric_capture(id) ON DELETE CASCADE,
    -- Canonical JSON of the promoted dimension values for this bucket
    -- (`{"oxplow.status":"passed"}`), '{}' when no dim is promoted. Built by the
    -- SAME `dim_value` the read uses — the bucketing is done in Rust precisely so
    -- there is never a second implementation of dim extraction to drift.
    dims_key TEXT NOT NULL,
    -- The decomposable components. `fact_count` is the bucket's fact count, NOT a
    -- subject count (a path may contribute many facts).
    fact_count INTEGER NOT NULL,
    value_sum REAL NOT NULL,
    value_min REAL,
    value_max REAL,
    numerator REAL NOT NULL DEFAULT 0,
    denominator REAL NOT NULL DEFAULT 0,
    PRIMARY KEY (measure_id, capture_id, dims_key)
) WITHOUT ROWID;

-- The read is "every bucket of this measure, oldest capture first", so the PK's
-- (measure_id, capture_id) prefix already serves it; this index serves the
-- capture-scoped drill-in and makes the CASCADE delete cheap.
CREATE INDEX idx_metric_cube_capture ON metric_cube(capture_id);

-- The fold's live state, MADE DURABLE — which facts are live right now. This is
-- what turns a replay into an increment: with it, a write costs O(the capture's
-- own facts) instead of O(all history), and the cube row is a FRESH aggregate
-- over the live state rather than arithmetic on the previous row.
--
-- That distinction is load-bearing, not stylistic. `state[N] = state[N-1] -
-- restated + facts` decrements fine for count/sum/num/den, but MIN/MAX are not
-- decrementable: evict the subject holding the max and the new max is
-- unrecoverable from the aggregate alone. `oxplow.tests.slowest_ms` is a `max`
-- over a per-subject measure, so a delta-maintained cube would have been silently
-- wrong for it. Re-aggregating live state is correct for every aggregation by
-- construction.
--
-- Sized by LIVE subjects (~2.5k per measure), not by history.
CREATE TABLE metric_live_fact (
    measure_id INTEGER NOT NULL REFERENCES measure(id) ON DELETE CASCADE,
    -- A stream is a worktree and the fold reconstructs ONE worktree's state
    -- (tsk98), so it keys the state alongside the producer: two worktrees running
    -- the same gauge share (producer, subject) keys and would otherwise evict
    -- each other.
    stream_id INTEGER NOT NULL,
    -- Partitioning by producer matters for the same reason: the 10 idiom gauges
    -- share `oxplow.ast_hit`, so without it a later gauge's capture would
    -- supersede an earlier gauge's facts for the same path.
    producer TEXT NOT NULL,
    -- The fold's eviction key: `path` for per-path, `subject_ref` for
    -- per-subject, or the repo-scalar sentinel for a path-less assertion.
    subject_key TEXT NOT NULL,
    fact_id INTEGER NOT NULL REFERENCES fact(id) ON DELETE CASCADE,
    PRIMARY KEY (measure_id, stream_id, producer, subject_key, fact_id)
) WITHOUT ROWID;

-- How far the cube has been built per (measure, stream). Without it, "no cube
-- rows for capture N" is AMBIGUOUS: it could mean the state was legitimately
-- empty at N (a real value-0 point) or that N simply has not been cubed yet (fall
-- back to the facts). Conflating those is how a materialized read reports zero
-- instead of admitting it does not know.
CREATE TABLE metric_cube_state (
    measure_id INTEGER NOT NULL REFERENCES measure(id) ON DELETE CASCADE,
    stream_id INTEGER NOT NULL,
    -- The newest capture folded into the cube. Captures after it are not cubed.
    last_capture_id INTEGER NOT NULL,
    last_captured_at TEXT NOT NULL,
    PRIMARY KEY (measure_id, stream_id)
) WITHOUT ROWID;

-- Promote `oxplow.status` — the cube's grain (tsk28's `promoted` flag, persisted
-- since V43 and inert until now, finally has its job).
--
-- Cardinality 2, and it is what `oxplow.tests.passed` / `oxplow.tests.failed`
-- filter on, so promoting it takes the test cube from 125 rows (capture only,
-- which cannot answer those two) to 152 rows that answer all five test specs.
-- `oxplow.test_suite` stays unpromoted on purpose: cardinality 234 would make the
-- same cube 18,918 rows for slicing no spec asks for. Promoting a dim later is a
-- cube REBUILD, not a schema change — the raw facts always keep every dim.
UPDATE dimension SET promoted = 1 WHERE key = 'oxplow.status';
