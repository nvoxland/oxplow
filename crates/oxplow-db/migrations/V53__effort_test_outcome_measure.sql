-- tsk38 — per-effort test-outcome scalars. "Tests failed" splits into
-- distinct questions the metric engine's cross-time collapse can't express
-- (max across runs / distinct-ever / count-of-red-runs), so the effort-
-- lifecycle producer (task_service.rs) materializes them once at effort close
-- as facts on this measure, sliced by the `oxplow.tests_stat` dimension
-- (`at_close` / `peak` / `distinct_failed` / `red_runs`). Read by the four
-- `oxplow.tests.{failed_at_close,peak_failed,distinct_failed,red_runs}` specs
-- (`producer_metrics.rs`).
--
-- NON-ADDITIVE with denominator 1, like the sibling per-close lifecycle
-- measures (`oxplow.cycle_time` / `oxplow.task_effort`, V47): the cross-time
-- collapse Σn/Σd is the MEAN per closed effort, never a lifetime sum. Additive
-- to the built-in catalog (append-only).

INSERT INTO dimension (key, label, value_type, subject_kind) VALUES
    ('oxplow.tests_stat', 'Test outcome stat', 'categorical', 'effort');

INSERT INTO measure (key, title, unit, subject_kind, temporal_semantics, component_role, created_at, updated_at) VALUES
    ('oxplow.effort_test_outcome', 'Effort test outcome', 'count', 'effort', 'non-additive', 'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z');
