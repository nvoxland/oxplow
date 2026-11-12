-- A badge for each task the request of the turn that took a snapshot worked
-- on: "completed: X" for a turn's end take, "in flight: X" for any other take
-- of a turn. Decorates `oxplow/snapshot` rows (Local History).
SELECT CAST(s.uri AS TEXT) AS uri,
       CAST((CASE WHEN x.completed THEN 'completed: ' ELSE 'in flight: ' END) || x.label AS TEXT) AS label
FROM (
  SELECT DISTINCT o.snapshot_id, o.trigger = 'turn_end' AS completed,
         coalesce(w.title, uri_param(rt.task, 'id'), 'Unlinked work') AS label
  FROM ref('snapshot_op') o
  JOIN ref('agent_turn') t ON t.id = o.turn_id
  LEFT JOIN ref('request_link') rt ON rt.request_id = t.request_id
  LEFT JOIN ref('task') w ON w.uri = rt.task
) x
JOIN ref('snapshot') s ON s.id = x.snapshot_id
ORDER BY s.id, x.completed DESC, x.label
