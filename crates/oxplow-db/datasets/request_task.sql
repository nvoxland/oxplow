-- Each task a request worked on, with the evidence: one row per
-- (request, task, basis), and the request's totals so far.
SELECT CAST(uri('oxplow/request', 'id', 'req' || rt.request_id) AS TEXT) AS request,
       rt.task_uri AS task, rt.basis,
       CAST(CASE WHEN rt.turn_id IS NULL THEN NULL
                 ELSE uri('oxplow/turn', 'id', 'trn' || rt.turn_id) END AS TEXT) AS turn,
       CAST((SELECT t.thread_id FROM source('agent_turn') t WHERE t.id = rt.turn_id) AS INT) AS thread_id,
       rt.at,
       CAST((SELECT COUNT(*) FROM source('agent_turn') t WHERE t.request_id = rt.request_id) AS INT) AS turns,
       CAST((SELECT MIN(t.started_at) FROM source('agent_turn') t WHERE t.request_id = rt.request_id) AS TEXT) AS started_at,
       CAST((SELECT MAX(t.ended_at) FROM source('agent_turn') t WHERE t.request_id = rt.request_id) AS TEXT) AS last_ended_at
  FROM source('request_task') rt
