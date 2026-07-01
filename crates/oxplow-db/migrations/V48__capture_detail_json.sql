-- T-E1 (tsk48, epic tsk12): the capture becomes the run. The verbatim per-run
-- detail payload (test suite/case tree, coverage per-file line-sets, analysis
-- findings) moves from the legacy `metric_finding` `*-detail` rows onto the
-- capture spine, as an envelope:
--   {"kind": "test-detail" | "coverage-detail" | "analysis-detail", "payload": {…}}
-- The observations panel and the read-time diff-coverage derivation read this;
-- the legacy metric_run/metric_sample/metric_finding cluster retires in T-E3.
ALTER TABLE metric_capture ADD COLUMN detail_json TEXT;
