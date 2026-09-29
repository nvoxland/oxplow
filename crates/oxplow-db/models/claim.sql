SELECT c.id, c.thread_id, c.task_id, c.effort_id, c.turn_id, c.statement, c.kind,
       c.evidence_ref,
       CASE
         WHEN c.evidence_ref IS NOT NULL THEN 1
         WHEN c.kind = 'tests_pass' AND c.effort_id IS NOT NULL AND EXISTS (
           SELECT 1 FROM (
             SELECT r.failed, r.total FROM ref('test_run') r
             WHERE r.effort_id = c.effort_id
             ORDER BY r.captured_at DESC, r.id DESC
             LIMIT 1
           ) latest
           WHERE latest.failed = 0 AND latest.total > 0
         ) THEN 1
         ELSE 0
       END AS verified,
       c.created_at
FROM source('claim') c
