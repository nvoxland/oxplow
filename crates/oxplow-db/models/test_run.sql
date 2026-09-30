SELECT c.id,
       c.stream_id,
       c.thread_id,
       COALESCE(
         (SELECT min(a.effort_id) FROM source('effort_attribution') a
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
       c.closest_vcs_rev,
       c.captured_at
FROM source('metric_capture') c
WHERE c.producer IN ('tests', 'test-run')
  AND json_extract(c.detail_json, '$.kind') = 'test-detail'
