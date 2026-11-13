-- A badge for each task the requests of a session's turns worked on, in the
-- order they were first worked on. Decorates `oxplow/agent_session` rows
-- (a thread's sessions).
SELECT CAST(s.uri AS TEXT) AS uri, CAST(x.title AS TEXT) AS label
FROM (
  SELECT t.agent_session_id AS session_id, w.title AS title, w.uri AS task, min(rt.at) AS first_at
  FROM ref('agent_turn') t
  JOIN ref('request_link') rt ON rt.request_id = t.request_id
  JOIN ref('task') w ON w.uri = rt.task
  WHERE t.agent_session_id IS NOT NULL
  GROUP BY t.agent_session_id, w.uri
) x
JOIN ref('agent_session') s ON s.id = x.session_id
ORDER BY s.id, x.first_at, x.task
