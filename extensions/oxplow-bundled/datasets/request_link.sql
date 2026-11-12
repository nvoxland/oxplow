-- The tasks each request worked on, in today's `v_request_task` shape, over
-- the `request_task` model. One row per (request,
-- task, basis), with the request's totals so far. `basis` is the model's:
-- `filed` and `edited` are core's `command`.
SELECT CAST(m.request AS TEXT) AS request,
       CAST(m.task AS TEXT) AS task,
       CAST(m.basis AS TEXT) AS basis,
       CAST(CASE WHEN m.turn_id IS NULL THEN NULL
                 ELSE uri('oxplow/turn', 'id', 'trn' || m.turn_id) END AS TEXT) AS turn,
       CAST((SELECT t.thread_id FROM ref('agent_turn') t WHERE t.id = m.turn_id) AS INT) AS thread_id,
       CAST(m.at AS TEXT) AS at,
       CAST((SELECT COUNT(*) FROM ref('agent_turn') t WHERE t.request_id = r.id) AS INT) AS turns,
       CAST((SELECT MIN(t.started_at) FROM ref('agent_turn') t WHERE t.request_id = r.id) AS TEXT) AS started_at,
       CAST((SELECT MAX(t.ended_at) FROM ref('agent_turn') t WHERE t.request_id = r.id) AS TEXT) AS last_ended_at,
       CAST(r.id AS INT) AS request_id
  FROM ref('request_task') m
  JOIN ref('request') r ON r.uri = m.request
