-- Files a request changed outside the area its task names (`v_oxplow_deviation`),
-- for the change of that request. `target` of a request's change is the
-- request's row id as text. Context only (`info`): the person may have
-- meant it.
-- It starts at the change: a join from `deviation` on the request would
-- work out every request's deviations before it narrowed to one change.
SELECT c.id AS change_id, 'oxplow/review_signal_scope' AS source, 'scope' AS area,
       'outside-task-area' AS signal, 'info' AS severity, f.path AS path,
       NULL AS line, NULL AS end_line, 'Outside the task''s area' AS title,
       'The task doesn''t name this file or its folder.' AS why, NULL AS uri
FROM ref('change') c
JOIN ref('change_file') f ON f.change_id = c.id
WHERE c.kind = 'request'
  AND f.path IN (SELECT d.path FROM ref('deviation') d WHERE d.request_id = CAST(c.target AS INTEGER))
