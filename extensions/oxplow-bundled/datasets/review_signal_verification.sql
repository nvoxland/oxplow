-- What a request's own runs say was verified, from the reports oxplow read
-- (never the agent's word): its latest test run failed, it ran no test at
-- all, and the changed lines its latest diff coverage found no test ran.
-- Each arm reads the change's own row and looks its runs up by its request
-- (no CTE shared by the arms: one read twice is computed for every change).
SELECT c.id AS change_id, 'oxplow/review_signal_verification' AS source, 'verification' AS area,
       'tests-failing' AS signal, 'high' AS severity, NULL AS path,
       NULL AS line, NULL AS end_line, 'Tests failing' AS title,
       printf('The latest test run (%s) had %d failing test%s.', r.command, r.failed,
              CASE r.failed WHEN 1 THEN '' ELSE 's' END) AS why,
       NULL AS uri
FROM ref('change') c
JOIN ref('test_run') r ON r.id = (
  SELECT r2.id FROM ref('agent_turn') t2 JOIN ref('test_run') r2 ON r2.turn_id = t2.id
   WHERE t2.request_id = CAST(c.target AS INTEGER)
   ORDER BY r2.captured_at DESC, r2.id DESC LIMIT 1)
WHERE c.kind = 'request' AND c.status = 'done' AND r.failed > 0
UNION ALL
SELECT c.id, 'oxplow/review_signal_verification', 'verification', 'no-test-run', 'medium',
       NULL, NULL, NULL, 'No tests ran',
       'No test run was seen in this request: nothing oxplow read checked the change.', NULL
FROM ref('change') c
WHERE c.kind = 'request' AND c.status = 'done'
  AND NOT EXISTS (SELECT 1 FROM ref('agent_turn') t JOIN ref('test_run') r ON r.turn_id = t.id
                   WHERE t.request_id = CAST(c.target AS INTEGER))
  AND EXISTS (SELECT 1 FROM ref('change_file') f WHERE f.change_id = c.id AND f.is_test = 0)
UNION ALL
SELECT c.id, 'oxplow/review_signal_verification', 'verification', 'uncovered-lines', 'medium',
       json_extract(file.value, '$.path'),
       json_extract(file.value, '$.uncoveredChangedLines[0]'), NULL,
       'Changed lines no test ran',
       printf('%d changed line%s in this file ran in no test (from line %d).',
              json_array_length(file.value, '$.uncoveredChangedLines'),
              CASE json_array_length(file.value, '$.uncoveredChangedLines') WHEN 1 THEN '' ELSE 's' END,
              json_extract(file.value, '$.uncoveredChangedLines[0]')),
       NULL
FROM ref('change') c
JOIN ref('request_observation') o ON o.seq = (
  SELECT max(o2.seq) FROM ref('request_observation') o2
   WHERE o2.request_id = CAST(c.target AS INTEGER) AND o2.kind = 'diff-coverage')
 AND o.request_id = CAST(c.target AS INTEGER) AND o.kind = 'diff-coverage'
JOIN json_each(o.payload_json, '$.files') AS file
WHERE c.kind = 'request' AND c.status = 'done'
  AND json_array_length(file.value, '$.uncoveredChangedLines') > 0
