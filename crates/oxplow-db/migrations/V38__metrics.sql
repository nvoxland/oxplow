-- Unified metric substrate (epic tsk213, phase P0).
--
-- One typed metric model replacing the bespoke `effort_observation` +
-- `code_quality_*` tables (dropped later, in P1 / P3b). A metric is a
-- deterministically-computable measure, logged DURABLY (it outlives the
-- effort that produced it), sliceable by conformed dimensions for reporting.
--
-- Design highlights (full rationale in the approved plan + .context/metrics.md):
--   * Typed kinds (gauge | findings | test | coverage | event) share a common
--     envelope; each projects >=1 scalar row into `metric_sample` (the narrow
--     durable fact table the explorer/targets/feedback read). Rich per-kind
--     structure lives in typed detail (`metric_finding`, etc.).
--   * TIME-PRIMARY: a sample is anchored by `captured_at` + `closest_git_version`
--     (+ `snapshot_id`). There is NO `effort_id` foreign key — efforts/commits
--     are time-range OVERLAYS (read from `task_effort`), so they can be garbage
--     collected without touching a single sample, a sample can fall in zero or
--     many efforts, and a diff metric stays interpretable via its `basis_ref`
--     baseline version after its effort is gone.
--   * Ratio metrics store numerator + denominator so roll-ups RE-AGGREGATE
--     correctly (coverage % by module is not a naive AVG of per-file %s).
--   * No keep-last-N prune, no CASCADE-with-effort. Stream is the hard
--     workspace scope (CASCADE); thread is a durable identity dimension
--     (SET NULL on delete so history survives).

-- The measure catalog. One row per known metric (built-in, user-global, or
-- project-defined). `key` is the stable, namespaced id (`oxplow.*` reserved).
CREATE TABLE metric_definition (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    key TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK (kind IN ('gauge', 'findings', 'test', 'coverage', 'event')),
    title TEXT NOT NULL,
    unit TEXT,
    direction TEXT NOT NULL DEFAULT 'neutral'
        CHECK (direction IN ('higher-better', 'lower-better', 'neutral')),
    default_agg TEXT NOT NULL DEFAULT 'last'
        CHECK (default_agg IN ('last', 'sum', 'avg', 'min', 'max')),
    grain TEXT CHECK (grain IS NULL OR grain IN ('effort', 'tree', 'file', 'entity')),
    basis TEXT NOT NULL DEFAULT 'absolute',
    producer TEXT,
    description TEXT,
    category TEXT,
    language TEXT,
    scope TEXT NOT NULL DEFAULT 'built-in'
        CHECK (scope IN ('built-in', 'global', 'project')),
    -- Declared conformed-dimension keys this metric carries (JSON array of
    -- `metric_dimension.key`s). Advisory metadata that powers the explorer's
    -- group-by + cross-metric drill-across.
    dimensions_json TEXT,
    target REAL,
    warn_at REAL,
    fail_at REAL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX idx_metric_definition_scope ON metric_definition(scope);
CREATE INDEX idx_metric_definition_language ON metric_definition(language, category);

-- Conformed-dimension catalog: each dimension registered once so two metrics
-- both tagging e.g. `language` mean the same thing and can be correlated.
CREATE TABLE metric_dimension (
    key TEXT PRIMARY KEY,
    label TEXT NOT NULL,
    value_type TEXT NOT NULL
        CHECK (value_type IN ('categorical', 'numeric', 'temporal', 'entity-ref')),
    -- For entity-ref dims: which subject kind they range over.
    subject_kind TEXT,
    -- Optional controlled value set (JSON array).
    vocabulary_json TEXT
);

-- Subject hierarchy (file -> module -> package -> repo) so file-grain metrics
-- roll up to module/repo. The one dimension that warrants a real dim table.
CREATE TABLE metric_subject (
    subject_kind TEXT NOT NULL,
    subject_ref TEXT NOT NULL,
    parent_kind TEXT,
    parent_ref TEXT,
    PRIMARY KEY (subject_kind, subject_ref)
);

-- A compute event that produces samples and/or findings (generalizes
-- `code_quality_scan`). Time/version anchored; effort is an overlay, never a
-- stored column. One run can feed many metrics. Raw events (page visits,
-- agent-asserted records) have no run (`metric_sample.run_id` is NULL).
CREATE TABLE metric_run (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    thread_id INTEGER REFERENCES threads(id) ON DELETE SET NULL,
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
    -- Branch the facts were captured on (e.g. `main`, `metrics-substrate`).
    -- Nullable: detached HEAD or a non-git capture has no branch.
    branch TEXT,
    git_version_exact INTEGER NOT NULL DEFAULT 0,
    started_at TEXT NOT NULL,
    ended_at TEXT
);

CREATE INDEX idx_metric_run_stream ON metric_run(stream_id);
CREATE INDEX idx_metric_run_producer ON metric_run(producer, started_at DESC);

-- The durable scalar fact table — the BI grain. Every kind projects >=1 row
-- here. NO `effort_id`: anchored by `captured_at` + `closest_git_version`.
CREATE TABLE metric_sample (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- NULL for run-less events / asserted records. SET NULL (not CASCADE) so a
    -- swept run never deletes the durable sample.
    run_id INTEGER REFERENCES metric_run(id) ON DELETE SET NULL,
    metric_id INTEGER NOT NULL REFERENCES metric_definition(id) ON DELETE CASCADE,
    value REAL NOT NULL,
    -- Ratio metrics keep their components so group-by roll-ups re-aggregate
    -- correctly instead of naive-averaging a percentage.
    numerator REAL,
    denominator REAL,
    -- PRIMARY axis: time + code-version (NOT effort).
    captured_at TEXT NOT NULL,
    snapshot_id INTEGER,
    closest_git_version TEXT,
    -- Branch the fact was captured on, when applicable (NULL = detached HEAD /
    -- non-git). A conformed dimension so metrics compare/filter across branches.
    branch TEXT,
    git_version_exact INTEGER NOT NULL DEFAULT 0,
    -- Baseline version for diff metrics (start snapshot/commit) — NOT an effort tie.
    basis_ref TEXT,
    -- Scope + durable identity dimensions.
    stream_id INTEGER NOT NULL REFERENCES streams(id) ON DELETE CASCADE,
    thread_id INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    -- Subject (generalizes "path"): file:… | symbol:… | pkg:… | module:… | model:… | page:…
    subject_kind TEXT,
    subject_ref TEXT,
    path TEXT,
    line INTEGER,
    -- Open author dimensions (severity | status | language | statistic | window).
    dims_json TEXT,
    provenance TEXT NOT NULL CHECK (provenance IN ('observed', 'asserted')),
    source TEXT NOT NULL
);

CREATE INDEX idx_metric_sample_metric_time ON metric_sample(metric_id, captured_at DESC);
CREATE INDEX idx_metric_sample_metric_version ON metric_sample(metric_id, closest_git_version);
CREATE INDEX idx_metric_sample_subject ON metric_sample(subject_kind, subject_ref);
CREATE INDEX idx_metric_sample_branch ON metric_sample(metric_id, branch);
CREATE INDEX idx_metric_sample_stream ON metric_sample(stream_id);
CREATE INDEX idx_metric_sample_run ON metric_sample(run_id);
CREATE INDEX idx_metric_sample_time ON metric_sample(captured_at);

-- Located detail for the `findings` kind (generalizes `code_quality_finding`):
-- lint hits, duplicate blocks, complexity hotspots, test cases, CVEs. Hangs off
-- a run (CASCADE — detail, not the durable time-series; the count projects to a
-- `metric_sample`).
CREATE TABLE metric_finding (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id INTEGER NOT NULL REFERENCES metric_run(id) ON DELETE CASCADE,
    metric_id INTEGER REFERENCES metric_definition(id) ON DELETE SET NULL,
    subject_kind TEXT,
    subject_ref TEXT,
    path TEXT,
    start_line INTEGER,
    end_line INTEGER,
    col INTEGER,
    kind TEXT NOT NULL,
    severity TEXT,
    rule TEXT,
    message TEXT,
    value REAL,
    extra_json TEXT
);

CREATE INDEX idx_metric_finding_run ON metric_finding(run_id);
CREATE INDEX idx_metric_finding_metric ON metric_finding(metric_id);
CREATE INDEX idx_metric_finding_kind ON metric_finding(kind, severity);
CREATE INDEX idx_metric_finding_path ON metric_finding(path);

-- Seed the built-in conformed dimensions. Author dims register additional rows.
INSERT INTO metric_dimension (key, label, value_type, subject_kind) VALUES
    ('time',        'Time',         'temporal',   NULL),
    ('stream',      'Stream',       'categorical', NULL),
    ('thread',      'Thread',       'categorical', NULL),
    ('effort',      'Effort',       'categorical', NULL),
    ('git_version', 'Git version',  'categorical', NULL),
    ('branch',      'Branch',       'categorical', NULL),
    ('subject',     'Subject',      'entity-ref',  NULL),
    ('model',       'Model',        'categorical', 'model'),
    ('agent',       'Agent',        'categorical', 'agent'),
    ('language',    'Language',     'categorical', NULL),
    ('severity',    'Severity',     'categorical', NULL),
    ('status',      'Status',       'categorical', NULL);
