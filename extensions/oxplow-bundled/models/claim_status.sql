-- Whether each claim is backed, by this review policy's rule: it cites
-- evidence (a run, a test, a file, or a person who looked), or it says
-- the tests pass and its effort's latest test run passed with at least
-- one test.
SELECT ref, effort_id, CAST(backed_by IS NOT NULL AS INTEGER) AS verified, backed_by
FROM (
SELECT c.ref,
       c.effort_id,
       CAST(CASE
         WHEN c.evidence_ref IS NOT NULL THEN 'evidence'
         WHEN c.kind = 'tests_pass' AND c.effort_id IS NOT NULL AND EXISTS (
           SELECT 1 FROM (
             SELECT r.failed, r.total FROM ref('test_run') r
             WHERE r.effort_id = c.effort_id
             ORDER BY r.captured_at DESC, r.id DESC
             LIMIT 1
           ) latest
           WHERE latest.failed = 0 AND latest.total > 0
         ) THEN 'latest run'
       END AS TEXT) AS backed_by
FROM ref('claim') c
)
