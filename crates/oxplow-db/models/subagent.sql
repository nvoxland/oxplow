-- Subagents of an agent session, from the events that bracket them and the
-- tool calls that name them. A subagent is not a session: it has no process
-- oxplow knows, only who made a call (`v_tool_call.subagent_id`). Claude
-- Code brackets one with `agent.subagent.started` / `.finished`; Codex and
-- opencode children appear only in their tool calls, so they are `unknown`.
WITH ids AS (
  SELECT agent_session_id, json_extract(payload, '$.subagent.id') AS id
    FROM source('event_log')
   WHERE type IN ('agent.subagent.started', 'agent.subagent.finished')
     AND agent_session_id IS NOT NULL
  UNION
  SELECT agent_session_id, subagent_id
    FROM source('agent_tool_call')
   WHERE subagent_id IS NOT NULL AND agent_session_id IS NOT NULL
), joined AS (
  SELECT i.agent_session_id, i.id,
         (SELECT min(e.at) FROM source('event_log') e
           WHERE e.type = 'agent.subagent.started' AND e.agent_session_id = i.agent_session_id
             AND json_extract(e.payload, '$.subagent.id') = i.id) AS started_at,
         (SELECT e.at FROM source('event_log') e
           WHERE e.type = 'agent.subagent.finished' AND e.agent_session_id = i.agent_session_id
             AND json_extract(e.payload, '$.subagent.id') = i.id
           ORDER BY e.seq LIMIT 1) AS finished_at,
         (SELECT e.id FROM source('event_log') e
           WHERE e.type = 'agent.subagent.finished' AND e.agent_session_id = i.agent_session_id
             AND json_extract(e.payload, '$.subagent.id') = i.id
           ORDER BY e.seq LIMIT 1) AS finished_event_id,
         coalesce(
           (SELECT json_extract(e.payload, '$.subagent.kind') FROM source('event_log') e
             WHERE e.type IN ('agent.subagent.started', 'agent.subagent.finished')
               AND e.agent_session_id = i.agent_session_id
               AND json_extract(e.payload, '$.subagent.id') = i.id
               AND json_extract(e.payload, '$.subagent.kind') IS NOT NULL
             ORDER BY e.seq LIMIT 1),
           (SELECT t.subagent_kind FROM source('agent_tool_call') t
             WHERE t.agent_session_id = i.agent_session_id AND t.subagent_id = i.id
               AND t.subagent_kind IS NOT NULL
             ORDER BY t.id LIMIT 1)) AS kind,
         (SELECT count(*) FROM source('agent_tool_call') t
           WHERE t.agent_session_id = i.agent_session_id AND t.subagent_id = i.id) AS calls,
         (SELECT max(t.at) FROM source('agent_tool_call') t
           WHERE t.agent_session_id = i.agent_session_id AND t.subagent_id = i.id) AS last_call_at,
         coalesce(
           (SELECT e.turn_id FROM source('event_log') e
             WHERE e.type = 'agent.subagent.started' AND e.agent_session_id = i.agent_session_id
               AND json_extract(e.payload, '$.subagent.id') = i.id
             ORDER BY e.seq LIMIT 1),
           (SELECT t.turn_id FROM source('agent_tool_call') t
             WHERE t.agent_session_id = i.agent_session_id AND t.subagent_id = i.id
             ORDER BY t.id LIMIT 1)) AS turn_id
    FROM ids i
)
SELECT CAST(j.id AS TEXT) AS id, CAST(j.kind AS TEXT) AS kind, j.agent_session_id, s.thread_id,
       j.turn_id,
       CAST(j.started_at AS TEXT) AS started_at, j.finished_at,
       j.calls,
       -- The newest of its start, its calls and its finish.
       CAST(max(coalesce(j.started_at, ''), coalesce(j.last_call_at, ''), coalesce(j.finished_at, ''))
            AS TEXT) AS last_activity_at,
       CAST(CASE WHEN j.finished_at IS NOT NULL THEN 'finished'
                 WHEN j.started_at IS NOT NULL THEN 'running'
                 ELSE 'unknown' END AS TEXT) AS state,
       j.finished_event_id,
       CAST(uri('oxplow/agent_session', 'id', 'ses' || j.agent_session_id) AS TEXT) AS session_ref,
       CAST(uri('oxplow/subagent', 'session', 'ses' || j.agent_session_id, 'agent', j.id) AS TEXT) AS ref
FROM joined j
JOIN source('agent_session') s ON s.id = j.agent_session_id
