-- Tests the change weakened: a test deleted, a test file that asserts less,
-- a test newly skipped. Weakening a test is how a failing check is made to
-- pass, so each is `high`.
SELECT fn.change_id, 'oxplow/review_signal_tests' AS source, 'tests' AS area,
       'test-deleted' AS signal, 'high' AS severity, fn.path,
       fn.start_line AS line, NULL AS end_line,
       'Test `' || CASE WHEN fn.container <> '' THEN fn.container || '::' ELSE '' END
         || fn.name || '` deleted' AS title,
       'The change removes this test.' AS why, fn.uri
FROM ref('change_function') fn
WHERE fn.is_test = 1 AND fn.status = 'deleted'
UNION ALL
SELECT t.change_id, 'oxplow/review_signal_tests', 'tests', 'fewer-assertions', 'high', t.path,
       NULL, NULL, 'Fewer assertions',
       printf('This file has %d → %d assertions.', t.assertions_before, t.assertions_after), NULL
FROM ref('change_test_file') t
WHERE t.assertions_after < t.assertions_before
UNION ALL
SELECT t.change_id, 'oxplow/review_signal_tests', 'tests', 'skip-added', 'high', t.path,
       NULL, NULL, 'Test skipped',
       printf('This file has %d → %d skip marker%s.', t.skips_before, t.skips_after,
              CASE t.skips_after WHEN 1 THEN '' ELSE 's' END), NULL
FROM ref('change_test_file') t
WHERE t.skips_after > t.skips_before
