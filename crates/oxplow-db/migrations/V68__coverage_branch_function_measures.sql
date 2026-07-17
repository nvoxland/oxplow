-- Branch & function coverage (tsk123). Line coverage overstates thoroughness —
-- a line executes but its `else` may never run, and a covered line says nothing
-- about whether every function ran. lcov / cobertura / jacoco reports already
-- carry branch and function/method hit counts; we only surfaced line-% before.
--
-- Two new per-file measures beside `oxplow.coverage` (line), same shape:
-- `%`, subject `file`, `semi-additive` (a run RESTATES the value, like the line
-- measure post-V50 — the headline is the latest capture's Σhit/Σfound, not a
-- history blend). Their facts carry num/den = hit/found so the engine re-derives
-- the ratio; a `ratio` producer spec (`producer_metrics.rs`) reads them.
-- Additive to the built-in catalog (append-only) — a new measure row is a
-- catalog INSERT, never a table rebuild, so no fact CASCADE fires.

INSERT INTO measure (key, title, unit, subject_kind, temporal_semantics, component_role, created_at, updated_at) VALUES
    ('oxplow.coverage.branch',   'Branch coverage',   '%', 'file', 'semi-additive', 'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z'),
    ('oxplow.coverage.function', 'Function coverage', '%', 'file', 'semi-additive', 'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z');
