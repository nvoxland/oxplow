-- What a request's own runs say was verified, from the reports oxplow read
-- (never the agent's word): its latest test run failed, it ran no test at
-- all, and the changed lines its latest diff coverage found no test ran.
WITH req AS (
  SELECT c.id AS change_id, CAST(c.target AS INTEGER) AS request_id
  FROM ref('change') c
  WHERE c.kind = 'request' AND c.status = 'done'
),
runs AS (
  SELECT req.change_id, r.id, r.failed, r.total, r.command,
         row_number() OVER (PARTITION BY req.change_id ORDER BY r.captured_at DESC, r.id DESC) AS nth
  FROM req
  JOIN ref('agent_turn') t ON t.request_id = req.request_id
  JOIN ref('test_run') r ON r.turn_id = t.id
),
coverage AS (
  SELECT req.change_id, o.payload_json,
         row_number() OVER (PARTITION BY req.change_id ORDER BY o.seq DESC) AS nth
  FROM req
  JOIN ref('request_observation') o ON o.request_id = req.request_id AND o.kind = 'diff-coverage'
)
SELECT change_id, 'oxplow/review_signal_verification' AS source, 'verification' AS area,
       'tests-failing' AS signal, 'high' AS severity, NULL AS path,
       NULL AS line, NULL AS end_line, 'Tests failing' AS title,
       printf('The latest test run (%s) had %d failing test%s.', command, failed,
              CASE failed WHEN 1 THEN '' ELSE 's' END) AS why,
       NULL AS uri
FROM runs WHERE nth = 1 AND failed > 0
UNION ALL
SELECT req.change_id, 'oxplow/review_signal_verification', 'verification', 'no-test-run', 'medium',
       NULL, NULL, NULL, 'No tests ran',
       'No test run was seen in this request: nothing oxplow read checked the change.', NULL
FROM req
WHERE NOT EXISTS (SELECT 1 FROM runs WHERE runs.change_id = req.change_id)
  AND EXISTS (SELECT 1 FROM ref('change_file') f WHERE f.change_id = req.change_id AND f.is_test = 0)
UNION ALL
SELECT coverage.change_id, 'oxplow/review_signal_verification', 'verification', 'uncovered-lines', 'medium',
       json_extract(file.value, '$.path'),
       json_extract(file.value, '$.uncoveredChangedLines[0]'), NULL,
       'Changed lines no test ran',
       printf('%d changed line%s in this file ran in no test (from line %d).',
              json_array_length(file.value, '$.uncoveredChangedLines'),
              CASE json_array_length(file.value, '$.uncoveredChangedLines') WHEN 1 THEN '' ELSE 's' END,
              json_extract(file.value, '$.uncoveredChangedLines[0]')),
       NULL
FROM coverage, json_each(coverage.payload_json, '$.files') AS file
WHERE coverage.nth = 1 AND json_array_length(file.value, '$.uncoveredChangedLines') > 0
