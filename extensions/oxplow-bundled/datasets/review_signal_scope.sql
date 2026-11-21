-- Files a request changed outside the area its task names (`v_oxplow_deviation`),
-- for the change of that request. `target` of a request's change is the
-- request's row id as text. Context only (`info`): the person may have
-- meant it.
SELECT c.id AS change_id, 'oxplow/review_signal_scope' AS source, 'scope' AS area,
       'outside-task-area' AS signal, 'info' AS severity, d.path AS path,
       NULL AS line, NULL AS end_line, 'Outside the task''s area' AS title,
       'The task doesn''t name this file or its folder.' AS why, NULL AS uri
FROM ref('deviation') d
JOIN ref('change') c ON c.kind = 'request' AND CAST(c.target AS INTEGER) = d.request_id
JOIN ref('change_file') f ON f.change_id = c.id AND f.path = d.path
