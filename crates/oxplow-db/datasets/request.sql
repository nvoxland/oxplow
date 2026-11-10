-- Each request: who started it and when (its first turn), what it holds
-- (its turns, their tool calls, tokens and test runs, through each turn),
-- and where it stands now. Every figure is read through the request's
-- turns (agent_turn.request_id), never stored.
--
-- state, read live:
--   waiting_on_person  an agent session whose latest turn is the request's
--                      waits on the person (v_agent_session_status
--                      `awaiting_user`: a final question, a permission
--                      prompt, a question tool);
--   running            else, one of its turns is open;
--   waiting_on_work    none is open, but a job one of its turns started
--                      has not reported back (and its session is alive);
--   finished           none of these.
SELECT r.id, r.thread_id,
       CAST(uri('oxplow/request', 'id', 'req' || r.id) AS TEXT) AS uri,
       CAST(CASE WHEN r.first_turn_id IS NULL THEN NULL
                 ELSE uri('oxplow/turn', 'id', 'trn' || r.first_turn_id) END AS TEXT) AS first_turn,
       f.source, f.prompt, f.started_at,
       CAST((SELECT count(*) FROM source('agent_turn') t WHERE t.request_id = r.id) AS INT) AS turns,
       CAST((SELECT count(DISTINCT t.thread_id) FROM source('agent_turn') t
              WHERE t.request_id = r.id) AS INT) AS threads,
       CAST((SELECT max(coalesce(t.ended_at, t.started_at)) FROM source('agent_turn') t
              WHERE t.request_id = r.id) AS TEXT) AS last_activity_at,
       CAST((SELECT count(*) FROM source('agent_turn') t
              JOIN source('agent_tool_call') c ON c.turn_id = t.id
             WHERE t.request_id = r.id AND c.state <> 'denied') AS INT) AS tool_calls,
       CAST((SELECT coalesce(sum(u.input_tokens + u.cache_creation_input_tokens
                                 + u.cache_read_input_tokens), 0)
               FROM source('agent_turn') t
               JOIN source('agent_token_usage') u ON u.turn_id = t.id
              WHERE t.request_id = r.id) AS INT) AS input_tokens,
       CAST((SELECT coalesce(sum(u.output_tokens), 0)
               FROM source('agent_turn') t
               JOIN source('agent_token_usage') u ON u.turn_id = t.id
              WHERE t.request_id = r.id) AS INT) AS output_tokens,
       CAST((SELECT count(*) FROM source('agent_turn') t
              JOIN source('agent_job') j ON j.started_turn_id = t.id
             WHERE t.request_id = r.id AND j.reported_at IS NULL AND j.lost_at IS NULL) AS INT)
         AS jobs_running,
       CAST(EXISTS (SELECT 1 FROM source('agent_turn') t
                     WHERE t.request_id = r.id AND t.cause_basis = 'matched') AS INT) AS matched,
       CAST(CASE
         -- An agent session whose latest turn is the request's waits on the
         -- person (a question it ended on, a permission prompt, a question
         -- tool): the status the rail shows, one mechanism.
         WHEN EXISTS (SELECT 1 FROM ref('agent_session_status') st
                       WHERE st.state = 'awaiting_user'
                         AND (SELECT t.request_id FROM source('agent_turn') t
                               WHERE t.agent_session_id = st.agent_session_id
                               ORDER BY t.id DESC LIMIT 1) = r.id) THEN 'waiting_on_person'
         WHEN EXISTS (SELECT 1 FROM source('agent_turn') t
                       WHERE t.request_id = r.id AND t.ended_at IS NULL) THEN 'running'
         WHEN EXISTS (SELECT 1 FROM source('agent_turn') t
                       JOIN source('agent_job') j ON j.started_turn_id = t.id
                      WHERE t.request_id = r.id AND j.reported_at IS NULL
                        AND j.lost_at IS NULL) THEN 'waiting_on_work'
         ELSE 'finished' END AS TEXT) AS state
  FROM source('request') r
  LEFT JOIN source('agent_turn') f ON f.id = r.first_turn_id
