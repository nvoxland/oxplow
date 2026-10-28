-- A metric's `expression:` (the canonical text of its aggregate-with-filter or formula):
-- what the manifest wrote, compiled to `aggregation` / `filter_json` / `formula`,
-- which the engine still reads. NULL for an entity metric.
ALTER TABLE metric_spec ADD COLUMN expression TEXT;
