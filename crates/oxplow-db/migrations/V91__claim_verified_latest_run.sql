-- `v_claim.verified` from the effort's LATEST test run (tsk366). V76 took
-- any clean `tests` report on the effort, so a later failing run still
-- verified the claim, a run claimed through attribution didn't count, and
-- runs without `oxplow.test_case` facts (no tests metric on) never did.
-- `v_test_run` resolves the run's effort (attribution first) and reads the
-- counts from the run's own payload.
DROP VIEW v_claim;
CREATE VIEW v_claim AS
SELECT c.id, c.thread_id, c.task_id, c.effort_id, c.statement, c.kind,
       c.evidence_ref,
       CASE
         WHEN c.evidence_ref IS NOT NULL THEN 1
         WHEN c.kind = 'tests_pass' AND c.effort_id IS NOT NULL AND EXISTS (
           SELECT 1 FROM (
             SELECT r.failed, r.total FROM v_test_run r
             WHERE r.effort_id = c.effort_id
             ORDER BY r.captured_at DESC, r.id DESC
             LIMIT 1
           ) latest
           WHERE latest.failed = 0 AND latest.total > 0
         ) THEN 1
         ELSE 0
       END AS verified,
       c.created_at
FROM claim c;
