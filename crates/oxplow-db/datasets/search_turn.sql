-- What site search indexes for each agent turn: the session's name and when
-- the turn started, and in the body what was asked and what the agent
-- answered. Turns on archived threads are included. `version` is when the
-- turn ended, else when it started: a turn's text is final once it ends.
-- `seq` is its row id: an insert-only recompute reads on from the highest
-- one indexed.
SELECT CAST(t.uri AS TEXT) AS uri,
       CAST(coalesce(s.display_name, 'turn') || ' · ' || t.started_at AS TEXT) AS title,
       CAST(coalesce(t.prompt, '') || char(10) || coalesce(t.answer, '') AS TEXT) AS body,
       CAST('str' || th.stream_id AS TEXT) AS stream_id,
       CAST(coalesce(t.ended_at, t.started_at) AS TEXT) AS version,
       t.id AS seq
FROM ref('agent_turn') t
LEFT JOIN ref('agent_session') s ON s.id = t.agent_session_id
LEFT JOIN source('threads') th ON th.id = t.thread_id
