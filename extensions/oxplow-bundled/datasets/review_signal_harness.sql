-- Files that decide what "passing" means: a test harness, a CI pipeline or a
-- lint config the change touched. Changing one is how a check is made to
-- pass without the code getting better, so each is `high`. One row per file
-- and kind, though several globs of the kind may match (`min` picks one).
SELECT f.change_id, 'oxplow/review_signal_harness' AS source, 'tests' AS area,
       CASE r.kind WHEN 'test-harness' THEN 'test-harness-changed'
                   WHEN 'ci' THEN 'ci-changed' ELSE 'lint-config-changed' END AS signal,
       'high' AS severity, f.path AS path, NULL AS line, NULL AS end_line,
       CASE r.kind WHEN 'test-harness' THEN 'Test harness changed'
                   WHEN 'ci' THEN 'CI config changed' ELSE 'Lint config changed' END AS title,
       printf('%s matches the %s rule %s, so what passing means may have changed.',
              f.path,
              CASE r.kind WHEN 'test-harness' THEN 'test harness' WHEN 'ci' THEN 'CI' ELSE 'lint config' END,
              min(r.glob)) AS why,
       NULL AS uri
FROM ref('change_file') f
JOIN ref('file_kind_rule') r ON r.kind IN ('test-harness', 'ci', 'lint-config') AND glob_match(r.glob, f.path)
GROUP BY f.change_id, f.path, r.kind
