-- Metric substrate redesign — the SPEC layer (epic tsk12, child tsk29).
--
-- Inverts V38's `metric_definition`: a metric is no longer a stored sample
-- STREAM (V38's `metric_definition` OWNED `metric_sample` rows, aggregation baked
-- in by the collector). A metric is now a SPEC computed over durable facts at read
-- time — a `source_measure` + an `aggregation` + an optional `filter`/`formula`,
-- plus presentation (direction + thresholds + display kind), evaluated by the
-- aggregation engine (`metric_engine.rs`).
--
-- ADDITIVE, strangler-consistent (mirrors V43): `metric_spec` lives BESIDE the old
-- `metric_definition`, which is still FK-referenced by the V38 `metric_sample`/
-- `metric_finding` tables, so the tree keeps compiling while reads (tsk26) move
-- over. The retire migration drops the whole V38 cluster once that flip lands. The
-- DB is wiped for this push, so there is no data to migrate.
--
-- Design (full rationale in the approved plan + .context/metrics.md):
--   * `source_measure` names the measure whose facts this metric aggregates; NULL
--     for a pure formula (derived) metric.
--   * `aggregation` is how the source facts combine WITHIN a capture; how the
--     resulting series then collapses across TIME is governed by the source
--     measure's `temporal_semantics`, NOT stored here.
--   * `filter_json` is a conjunctive predicate over facts (min_value / severity /
--     dim equality) — this is what turns a raw measure into a count-over-threshold.
--   * `formula` references other metric keys for a constrained derived metric
--     (decision #8: ratios + a few binary ops, no general DSL).
--   * Presentation (direction / target / warn_at / fail_at / display_kind) is
--     read-time only — severity/threshold-state are DERIVED from `value` × these,
--     never stored on a fact.

CREATE TABLE metric_spec (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL UNIQUE,               -- namespaced; `oxplow.*` reserved
    title TEXT NOT NULL,
    unit TEXT,
    -- The measure whose facts this metric aggregates. NULL for a formula metric
    -- (derived purely from other metrics via `formula`).
    source_measure TEXT,
    -- How the source measure's facts combine WITHIN a capture. (Cross-time collapse
    -- is the source measure's `temporal_semantics`, applied by the engine.)
    aggregation TEXT NOT NULL DEFAULT 'last'
        CHECK (aggregation IN
            ('count', 'count_distinct', 'sum', 'avg', 'min', 'max', 'last', 'p95', 'ratio')),
    -- Conjunctive predicate over facts (min_value / severity / dim equality), JSON.
    -- This is what makes a metric a count-over-threshold rather than a raw measure.
    filter_json TEXT,
    -- Derived-metric formula referencing other metric keys ({op, left, right}); NULL
    -- for a base metric. Mutually informative with `source_measure`.
    formula TEXT,
    -- Conformed dims this metric may be sliced by (JSON array of dimension keys).
    sliceable_dims_json TEXT,
    -- Presentation: how a good/bad reading is derived + rendered at READ time.
    direction TEXT NOT NULL DEFAULT 'neutral'
        CHECK (direction IN ('higher-better', 'lower-better', 'neutral')),
    target REAL,
    warn_at REAL,
    fail_at REAL,
    description TEXT,
    category TEXT,
    language TEXT,
    scope TEXT NOT NULL DEFAULT 'built-in'
        CHECK (scope IN ('built-in', 'global', 'project')),
    -- Read-time presentation kind (gauge | findings | test | coverage | event).
    display_kind TEXT NOT NULL DEFAULT 'gauge'
        CHECK (display_kind IN ('gauge', 'findings', 'test', 'coverage', 'event')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_metric_spec_scope ON metric_spec(scope);
CREATE INDEX idx_metric_spec_measure ON metric_spec(source_measure);
CREATE INDEX idx_metric_spec_language ON metric_spec(language, category);

-- No seed rows: specs are seeded from resolved config (built-in + global + project)
-- when read-flip (tsk26) repoints seeding onto specs. The engine is exercised in
-- tests with hand-inserted specs until then.
