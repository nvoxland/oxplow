-- Metric facts store each distinct subject, path and dimension set once.
-- A fact repeated the same text on every row (a test's name, a file's
-- path, `{"oxplow.status":"passed","oxplow.test_suite":…}`) — ~290 bytes
-- a fact with its indexes, the subject index alone 878 MB of a 4 GB
-- database — while the distinct values number in the thousands. A fact
-- now carries their ids.
CREATE TABLE fact_subject (
    id INTEGER PRIMARY KEY,
    kind TEXT,
    ref TEXT,
    UNIQUE (kind, ref)
);
CREATE TABLE fact_path (
    id INTEGER PRIMARY KEY,
    path TEXT NOT NULL UNIQUE
);
CREATE TABLE fact_dims (
    id INTEGER PRIMARY KEY,
    json TEXT NOT NULL UNIQUE
);

INSERT INTO fact_subject (kind, ref)
SELECT DISTINCT subject_kind, subject_ref FROM fact
 WHERE subject_kind IS NOT NULL OR subject_ref IS NOT NULL;
INSERT INTO fact_path (path) SELECT DISTINCT path FROM fact WHERE path IS NOT NULL;
INSERT INTO fact_dims (json) SELECT DISTINCT dims_json FROM fact WHERE dims_json IS NOT NULL;

-- The cube's live state points at facts by id; the table is rebuilt
-- under it, so the cube rebuilds from scratch (and a build in flight is
-- fenced).
DELETE FROM metric_live_fact;
DELETE FROM metric_cube;
DELETE FROM metric_cube_state;
UPDATE metric_cube_epoch SET epoch = epoch + 1;

CREATE TABLE fact_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    capture_id INTEGER NOT NULL REFERENCES metric_capture(id) ON DELETE CASCADE,
    measure_id INTEGER NOT NULL REFERENCES measure(id) ON DELETE CASCADE,
    value REAL NOT NULL,
    -- Ratio components, so roll-ups re-aggregate as Σnum/Σden (never naive-AVG).
    numerator REAL,
    denominator REAL,
    -- What the fact is about (`fact_subject`: kind + logical id) and where
    -- it was at capture (`fact_path` + `line`).
    subject_id INTEGER REFERENCES fact_subject(id),
    path_id INTEGER REFERENCES fact_path(id),
    line INTEGER,
    -- Reported finding metadata (lint/CVE); NULL for pure measurements.
    severity TEXT,
    rule TEXT,
    detail TEXT,
    -- Open conformed-dimension tail (`fact_dims`: its JSON, keyed by
    -- namespaced dimension key).
    dims_id INTEGER REFERENCES fact_dims(id)
);
INSERT INTO fact_new
    (id, capture_id, measure_id, value, numerator, denominator, subject_id, path_id,
     line, severity, rule, detail, dims_id)
SELECT f.id, f.capture_id, f.measure_id, f.value, f.numerator, f.denominator, s.id, p.id,
       f.line, f.severity, f.rule, f.detail, d.id
  FROM fact f
  LEFT JOIN fact_subject s ON s.kind IS f.subject_kind AND s.ref IS f.subject_ref
  LEFT JOIN fact_path p ON p.path = f.path
  LEFT JOIN fact_dims d ON d.json = f.dims_json;
DROP TABLE fact;
ALTER TABLE fact_new RENAME TO fact;

CREATE INDEX idx_fact_measure_capture ON fact(measure_id, capture_id);
CREATE INDEX idx_fact_subject ON fact(subject_id);
CREATE INDEX idx_fact_capture ON fact(capture_id);
CREATE INDEX idx_fact_measure_path ON fact(measure_id, path_id);
