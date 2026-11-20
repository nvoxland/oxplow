-- Tests the change weakened. Weakening a test is how a failing check is
-- made to pass, so each is `high`. The counts are heuristics over a file's
-- text, so they are read only in test files (`v_change_file.is_test`): a
-- `.skip(1)` or a dropped `.expect()` in other code is no test.
WITH files AS (
  SELECT f.change_id, f.path, f.status, t.tests_before, t.tests_after,
         t.assertions_before, t.assertions_after, t.skips_before, t.skips_after
  FROM ref('change_file') f
  JOIN ref('change_test_file') t ON t.change_id = f.change_id AND t.path = f.path
  WHERE f.is_test = 1
)
-- A test file deleted: one row for the file, not one per test.
SELECT change_id, 'oxplow/review_signal_tests' AS source, 'tests' AS area,
       'test-file-deleted' AS signal, 'high' AS severity, path,
       NULL AS line, NULL AS end_line, 'Test file deleted' AS title,
       printf('The change deletes this file and its %d test%s.', tests_before,
              CASE tests_before WHEN 1 THEN '' ELSE 's' END) AS why,
       NULL AS uri
FROM files WHERE status = 'deleted'
UNION ALL
-- A test deleted where the file now has fewer tests: a renamed test, or a
-- helper deleted beside a new test, keeps the count and isn't flagged.
SELECT fn.change_id, 'oxplow/review_signal_tests', 'tests', 'test-deleted', 'high', fn.path,
       fn.start_line, NULL,
       'Test deleted: ' || CASE WHEN fn.container <> '' THEN fn.container || '::' ELSE '' END || fn.name,
       printf('The file has %d → %d tests.', files.tests_before, files.tests_after),
       fn.uri
FROM ref('change_function') fn
JOIN files ON files.change_id = fn.change_id AND files.path = fn.path
WHERE fn.is_test = 1 AND fn.status = 'deleted'
  AND files.status <> 'deleted' AND files.tests_after < files.tests_before
UNION ALL
SELECT change_id, 'oxplow/review_signal_tests', 'tests', 'fewer-assertions', 'high', path,
       NULL, NULL, 'Fewer assertions',
       printf('This file has %d → %d assertions.', assertions_before, assertions_after), NULL
FROM files WHERE status <> 'deleted' AND assertions_after < assertions_before
UNION ALL
SELECT change_id, 'oxplow/review_signal_tests', 'tests', 'skip-added', 'high', path,
       NULL, NULL, 'Test skipped',
       printf('This file has %d → %d skip marker%s.', skips_before, skips_after,
              CASE skips_after WHEN 1 THEN '' ELSE 's' END), NULL
FROM files WHERE skips_after > skips_before
