-- Measures and metric specs an extension declares (tsk311). Their scope is
-- stored as 'global' (the V43/V44 CHECK allows only built-in / global /
-- project, and rebuilding these tables would cascade-delete every fact — see
-- V54) with the owning extension here; the store reads them back as
-- `extension:<name>`. Pruning keys on this column.
ALTER TABLE measure ADD COLUMN extension TEXT;
ALTER TABLE metric_spec ADD COLUMN extension TEXT;

DROP VIEW v_metric_spec;
CREATE VIEW v_metric_spec AS
SELECT key, title, unit, source_measure, aggregation, direction, target,
       warn_at, fail_at, description, category, language,
       CASE WHEN extension IS NOT NULL THEN 'extension' ELSE scope END AS scope,
       display_kind, extension
FROM metric_spec;
