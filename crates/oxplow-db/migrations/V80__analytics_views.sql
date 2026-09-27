-- Substrate views the analytics extension (and agents) read (tsk306):
-- metric definitions, nudges, code-quality scans/findings, dashboards, and
-- two tables core refreshes per effort from the metric engine (deltas and
-- observations), since lenses never compute.

CREATE VIEW v_metric_spec AS
SELECT key, title, unit, source_measure, aggregation, direction, target,
       warn_at, fail_at, description, category, language, scope, display_kind
FROM metric_spec;

CREATE VIEW v_agent_nudge AS
SELECT id, thread_id, effort_id, kind, message, trigger, created_at
FROM agent_nudge;

CREATE VIEW v_code_quality_scan AS
SELECT id, tool, scope, status, started_at, ended_at, error,
       tree_version_kind, tree_version_value, file_filter
FROM code_quality_scan;

CREATE VIEW v_code_quality_finding AS
SELECT f.id, f.scan_id, s.tool, f.path, f.start_line, f.end_line, f.kind,
       f.metric_value, f.extra_json, s.started_at AS scanned_at
FROM code_quality_finding f
JOIN code_quality_scan s ON s.id = f.scan_id;

CREATE VIEW v_dashboard AS
SELECT id, title, sort_index, created_at, updated_at FROM dashboard;

CREATE VIEW v_dashboard_item AS
SELECT id, dashboard_id, sort_index, kind, metric_key, options_json
FROM dashboard_item;

-- Per-effort metric deltas, as the metric engine computes them
-- (collection::effort_metric_deltas). Replaced wholesale per effort.
CREATE TABLE effort_metric_delta (
    effort_id INTEGER NOT NULL REFERENCES task_effort(id) ON DELETE CASCADE,
    key TEXT NOT NULL,
    title TEXT NOT NULL,
    unit TEXT,
    direction TEXT NOT NULL,
    kind TEXT NOT NULL,
    category TEXT,
    language TEXT,
    agg TEXT NOT NULL,
    baseline REAL,
    current REAL NOT NULL,
    delta REAL,
    changed INTEGER NOT NULL,
    attributed_files INTEGER,
    sample_count INTEGER NOT NULL,
    target REAL,
    warn_at REAL,
    fail_at REAL,
    crossing TEXT,
    latest_capture_id INTEGER,
    refreshed_at TEXT NOT NULL,
    PRIMARY KEY (effort_id, key)
);

CREATE VIEW v_effort_metric_delta AS
SELECT effort_id, key, title, unit, direction, kind, category, language, agg,
       baseline, current, delta, changed, attributed_files, sample_count,
       target, warn_at, fail_at, crossing, latest_capture_id, refreshed_at
FROM effort_metric_delta;

-- Per-effort evidence rows (tests run, diff coverage, analysis), rebuilt from
-- the effort's claimed captures (collection::effort_observations_from_metrics).
CREATE TABLE effort_observation_row (
    effort_id INTEGER NOT NULL REFERENCES task_effort(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    kind TEXT NOT NULL,
    provenance TEXT NOT NULL,
    source TEXT NOT NULL,
    metric_value REAL,
    payload_json TEXT,
    local_snapshot_id INTEGER,
    created_at TEXT NOT NULL,
    PRIMARY KEY (effort_id, seq)
);

CREATE VIEW v_effort_observation AS
SELECT effort_id, seq, kind, provenance, source, metric_value, payload_json,
       local_snapshot_id, created_at
FROM effort_observation_row;
