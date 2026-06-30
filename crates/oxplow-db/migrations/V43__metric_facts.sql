-- Metric substrate redesign — the durable atomic FACT layer (epic tsk12, child
-- tsk13). Inverts the V38 model: facts (today's ephemeral `metric_finding`) become
-- the durable source of truth; metrics become aggregation/formula specs computed
-- over them; the per-metric `metric_sample` stream demotes to a rebuildable cache.
--
-- This migration is ADDITIVE: it creates the new fact layer ALONGSIDE the V38
-- tables so the tree keeps compiling while producers (tsk14) and reads (tsk16) are
-- moved over. A later cleanup migration drops the obsolete V38 tables
-- (`metric_sample`, `metric_finding`, `metric_dimension`, `metric_subject`, and
-- the old `metric_run`/`metric_definition` shapes). The DB is wiped for this push,
-- so there is no data to migrate.
--
-- Design (full rationale in the approved plan + .context/metrics.md):
--   * THREE namespaced catalogs — `measure` (fact types), `dimension`,
--     `metric_definition` (the spec; reworked later). `oxplow.*` is reserved.
--   * `metric_capture` (renamed from `metric_run`) is the ONE context row: it
--     carries all when/where/who/effort/trust metadata. Facts hold ONLY the
--     measurement — no duplicated spine.
--   * `fact.capture_id` is NOT NULL (every fact has a capture). Captures are
--     DURABLE (the facts' context lives on them); GC of an effort SET-NULLs the
--     capture's `effort_id` but never deletes a fact.
--   * Conformed dims get a column/index (the catalog has teeth); the long tail
--     lives in `fact.dims_json`, queried via `json_extract` / expression indexes.

-- The catalog of fact TYPES — what a collector may emit. Namespaced; declared
-- before a fact may reference it (declare-to-collect, enforced in the producers).
CREATE TABLE measure (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL UNIQUE,                  -- namespaced; `oxplow.*` reserved
    title TEXT NOT NULL,
    unit TEXT,
    -- The grain's subject kind (symbol | file | test | dependency | model | …).
    subject_kind TEXT,
    -- Additivity OVER TIME — the BI semi-additive distinction. A snapshot measure
    -- (complexity, todo count, coverage) is semi-additive (sum across subjects,
    -- last/avg across time); an event measure (tokens, lint hits) is additive
    -- (sum incl. time); a ratio (coverage %) is non-additive (re-derive Σn/Σd).
    temporal_semantics TEXT NOT NULL DEFAULT 'semi-additive'
        CHECK (temporal_semantics IN ('additive', 'semi-additive', 'non-additive')),
    -- For ratio bases: whether this measure is the numerator/denominator of a
    -- derived ratio metric. Most measures are `none`.
    component_role TEXT NOT NULL DEFAULT 'none'
        CHECK (component_role IN ('none', 'numerator', 'denominator')),
    scope TEXT NOT NULL DEFAULT 'built-in'
        CHECK (scope IN ('built-in', 'global', 'project')),
    description TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- Conformed-dimension catalog (supersedes `metric_dimension`). A dimension means
-- the same thing to every fact that carries it → cross-metric drill-across.
-- `promoted = 1` once a generated column + expression index exists on `fact`.
CREATE TABLE dimension (
    key TEXT PRIMARY KEY,                      -- namespaced; `oxplow.*` reserved
    label TEXT NOT NULL,
    value_type TEXT NOT NULL
        CHECK (value_type IN ('categorical', 'numeric', 'temporal', 'entity-ref')),
    subject_kind TEXT,                         -- for entity-ref dims
    vocabulary_json TEXT,                      -- optional controlled value set
    scope TEXT NOT NULL DEFAULT 'built-in'
        CHECK (scope IN ('built-in', 'global', 'project')),
    promoted INTEGER NOT NULL DEFAULT 0
);

-- Subject hierarchy (file → package → repo; custom kinds e.g. dependency → repo),
-- now exercised for roll-up. Supersedes `metric_subject`.
CREATE TABLE subject (
    subject_kind TEXT NOT NULL,
    subject_ref TEXT NOT NULL,
    parent_kind TEXT,
    parent_ref TEXT,
    label TEXT,
    PRIMARY KEY (subject_kind, subject_ref)
);

-- The capture/context row — renamed from `metric_run`, now the SOLE home of every
-- per-event attribute (when/where/who/effort/trust). One capture produces >=1
-- facts that share this context. Durable: not swept independently of its facts.
CREATE TABLE metric_capture (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    thread_id INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    -- The PRODUCING effort (provenance), stamped only when unambiguous (single
    -- open effort or explicit task id), else NULL; backfilled by the attribution
    -- ledger at close. SET NULL on GC: the capture (and its facts) outlive the
    -- effort. NOT the reporting overlay (that stays time-window + claim ledger).
    effort_id INTEGER REFERENCES task_effort(id) ON DELETE SET NULL,
    producer TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'done'
        CHECK (status IN ('running', 'done', 'failed')),
    error TEXT,
    scope TEXT,
    trigger TEXT,
    basis_ref TEXT,
    provenance TEXT NOT NULL CHECK (provenance IN ('observed', 'asserted')),
    source TEXT NOT NULL,
    snapshot_id INTEGER,
    closest_git_version TEXT,
    git_version_exact INTEGER NOT NULL DEFAULT 0,
    branch TEXT,
    captured_at TEXT NOT NULL,
    ended_at TEXT
);

CREATE INDEX idx_metric_capture_stream ON metric_capture(stream_id);
CREATE INDEX idx_metric_capture_captured_at ON metric_capture(captured_at);
CREATE INDEX idx_metric_capture_branch ON metric_capture(branch);
CREATE INDEX idx_metric_capture_version ON metric_capture(closest_git_version);
CREATE INDEX idx_metric_capture_effort ON metric_capture(effort_id);
CREATE INDEX idx_metric_capture_producer ON metric_capture(producer, captured_at DESC);
CREATE INDEX idx_metric_capture_trigger ON metric_capture(trigger, captured_at);
CREATE INDEX idx_metric_capture_thread ON metric_capture(thread_id);

-- The durable atomic MEASUREMENT (folds V38's `metric_sample` + `metric_finding`).
-- `capture_id` is NOT NULL: all when/where/who context is reached via the capture.
-- `severity`/`rule`/`detail` are the optional REPORTED finding metadata (lint);
-- NULL for pure measurements, whose severity is derived at read from thresholds.
CREATE TABLE fact (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    capture_id INTEGER NOT NULL REFERENCES metric_capture(id) ON DELETE CASCADE,
    measure_id INTEGER NOT NULL REFERENCES measure(id) ON DELETE CASCADE,
    value REAL NOT NULL,
    -- Ratio components, so roll-ups re-aggregate as Σnum/Σden (never naive-AVG).
    numerator REAL,
    denominator REAL,
    -- Subject + location-at-capture (per-fact, unlike the version which is the
    -- capture's). `subject_ref` is the logical id; `path`/`line` the coordinate.
    subject_kind TEXT,
    subject_ref TEXT,
    path TEXT,
    line INTEGER,
    -- Reported finding metadata (lint/CVE); NULL for pure measurements.
    severity TEXT,
    rule TEXT,
    detail TEXT,
    -- Open conformed-dimension tail, keyed by namespaced dimension key.
    dims_json TEXT
);

CREATE INDEX idx_fact_measure_capture ON fact(measure_id, capture_id);
CREATE INDEX idx_fact_subject ON fact(subject_kind, subject_ref);
CREATE INDEX idx_fact_capture ON fact(capture_id);

-- Seed the built-in conformed dimensions (namespaced). Author dims register more.
INSERT INTO dimension (key, label, value_type, subject_kind) VALUES
    ('oxplow.language', 'Language',  'categorical', NULL),
    ('oxplow.severity', 'Severity',  'categorical', NULL),
    ('oxplow.status',   'Status',    'categorical', NULL),
    ('oxplow.branch',   'Branch',    'categorical', NULL),
    ('oxplow.model',    'Model',     'categorical', 'model'),
    ('oxplow.agent',    'Agent',     'categorical', 'agent'),
    ('oxplow.package',  'Package',   'categorical', NULL),
    ('oxplow.test_suite', 'Test suite', 'categorical', NULL);

-- Seed the built-in measures. `temporal_semantics` defaults are sensible starting
-- points; the producers (tsk14) refine per measure as they wire real facts.
INSERT INTO measure (key, title, unit, subject_kind, temporal_semantics, component_role, created_at, updated_at) VALUES
    ('oxplow.complexity',       'Cyclomatic complexity', 'count', 'symbol',     'semi-additive', 'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z'),
    ('oxplow.fn_length',        'Function length',       'lines', 'symbol',     'semi-additive', 'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z'),
    ('oxplow.parameter_count',  'Parameter count',       'count', 'symbol',     'semi-additive', 'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z'),
    ('oxplow.todo',             'TODO markers',          'count', 'file',       'semi-additive', 'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z'),
    ('oxplow.coverage',         'Line coverage',         '%',     'file',       'non-additive',  'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z'),
    ('oxplow.test_case',        'Test case',             'count', 'test',       'additive',      'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z'),
    ('oxplow.lint_hit',         'Static-analysis hit',   'count', 'file',       'additive',      'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z'),
    ('oxplow.duplicate_lines',  'Duplicated lines',      'lines', 'symbol',     'semi-additive', 'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z'),
    ('oxplow.tokens',           'Agent tokens',          'count', 'model',      'additive',      'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z'),
    ('oxplow.cycle_time',       'Effort cycle time',     'ms',    'effort',     'additive',      'none', '1970-01-01T00:00:00.000000Z', '1970-01-01T00:00:00.000000Z');
