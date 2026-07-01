-- Epic tsk12 / child T-B (producer specs + fact enrichment). The always-on
-- producers (tokens, turns, tests, coverage, analysis, lifecycle, nudges) invert
-- to facts on built-in measures, aggregated by a producer `metric_spec`
-- (`producer_metrics.rs::builtin_producer_specs`). Most reuse the V43 measures
-- (`oxplow.tokens`/`oxplow.cycle_time`/`oxplow.test_case`/`oxplow.coverage`/
-- `oxplow.lint_hit`), sliced by CONFORMED dimensions rather than extra measures
-- (decision #4): token in/out by `oxplow.token_kind`, test pass/fail by the
-- existing `oxplow.status`, analysis by `oxplow.severity`.
--
-- This migration adds only what V43 lacks: the `oxplow.token_kind` dimension and
-- the three producer measures with no existing home (agent turns, efforts-per-
-- task, nudges). Additive to V43's built-in catalog (keeps it append-only).

INSERT INTO dimension (key, label, value_type, subject_kind) VALUES
    ('oxplow.token_kind', 'Token kind', 'categorical', NULL);

INSERT INTO measure (key, title, unit, subject_kind, temporal_semantics, component_role, created_at, updated_at) VALUES
    ('oxplow.turn',        'Agent turns',      'count', 'model',  'additive',      'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z'),
    ('oxplow.task_effort', 'Efforts per task', 'count', 'task',   'semi-additive', 'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z'),
    ('oxplow.nudge',       'Nudges fired',     'count', NULL,     'additive',      'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z');
