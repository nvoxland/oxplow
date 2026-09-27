-- Test runs as the semantic layer reads them (tsk320). A run IS its
-- metric_capture (producer `tests`, or `test-run` for a run record that
-- measured nothing); its verbatim payload is `detail_json` =
-- {"kind":"test-detail","payload":{command, exitCode, durationMs, passed,
-- failed, skipped, total, suites:[{name, cases:[{classname, name, status,
-- timeMs}]}]}}. Cases come from that payload rather than the
-- `oxplow.test_case` facts, which are skipped when no tests metric is on.

CREATE VIEW v_test_run AS
SELECT c.id,
       c.stream_id,
       c.thread_id,
       COALESCE(
         (SELECT min(a.effort_id) FROM effort_attribution a
          WHERE a.kind = 'run' AND a.ref = 'run:' || c.id AND a.state = 'claimed'),
         c.effort_id) AS effort_id,
       json_extract(c.detail_json, '$.payload.command') AS command,
       json_extract(c.detail_json, '$.payload.exitCode') AS exit_code,
       json_extract(c.detail_json, '$.payload.passed') AS passed,
       json_extract(c.detail_json, '$.payload.failed') AS failed,
       json_extract(c.detail_json, '$.payload.skipped') AS skipped,
       json_extract(c.detail_json, '$.payload.total') AS total,
       json_extract(c.detail_json, '$.payload.durationMs') AS duration_ms,
       c.provenance,
       c.source,
       c.branch,
       c.closest_git_version,
       c.captured_at
FROM metric_capture c
WHERE c.producer IN ('tests', 'test-run')
  AND json_extract(c.detail_json, '$.kind') = 'test-detail';

CREATE VIEW v_test_case AS
SELECT c.id AS run_id,
       json_extract(s.value, '$.name') AS suite,
       json_extract(t.value, '$.classname') AS classname,
       json_extract(t.value, '$.name') AS name,
       json_extract(t.value, '$.status') AS status,
       json_extract(t.value, '$.timeMs') AS time_ms
FROM metric_capture c,
     json_each(c.detail_json, '$.payload.suites') s,
     json_each(s.value, '$.cases') t
WHERE c.producer IN ('tests', 'test-run')
  AND json_extract(c.detail_json, '$.kind') = 'test-detail';
