-- Subagents of an agent session, from the events that bracket them and the
-- tool calls that name them. A subagent is not a session: it has no process
-- oxplow knows, only who made a call (`v_tool_call.subagent_id`). Claude
-- Code brackets one with `agent.subagent.started` / `.finished`; Codex and
-- opencode children appear only in their tool calls, so they are `unknown`.
-- One pass over the subagent events and one over the calls that name a
-- subagent, each grouped by (session, subagent); never a subquery per
-- subagent (its id is inside the event's payload, and no index has a call's
-- subagent): that took thousands of subagents past `query_sql`'s 5 s.
WITH ev AS (
  SELECT agent_session_id, json_extract(payload, '$.subagent.id') AS id, type, at, seq, turn_id,
         json_extract(payload, '$.subagent.kind') AS kind
    FROM source('event_log')
   WHERE type IN ('agent.subagent.started', 'agent.subagent.finished')
     AND agent_session_id IS NOT NULL
), ev_firsts AS (
  SELECT agent_session_id, id,
         min(CASE WHEN type = 'agent.subagent.started' THEN at END) AS started_at,
         min(CASE WHEN type = 'agent.subagent.started' THEN seq END) AS start_seq,
         min(CASE WHEN type = 'agent.subagent.finished' THEN seq END) AS finish_seq,
         min(CASE WHEN kind IS NOT NULL THEN seq END) AS kind_seq
    FROM ev
   WHERE id IS NOT NULL
   GROUP BY agent_session_id, id
), calls AS (
  SELECT agent_session_id, subagent_id AS id, count(*) AS calls, max(at) AS last_call_at,
         min(id) AS first_call, min(CASE WHEN subagent_kind IS NOT NULL THEN id END) AS kind_call
    FROM source('agent_tool_call')
   WHERE subagent_id IS NOT NULL AND agent_session_id IS NOT NULL
   GROUP BY agent_session_id, subagent_id
), ids AS (
  SELECT agent_session_id, id FROM ev
  UNION
  SELECT agent_session_id, id FROM calls
), joined AS (
  SELECT i.agent_session_id, i.id, f.started_at,
         fin.at AS finished_at, fin.id AS finished_event_id,
         coalesce(json_extract(k.payload, '$.subagent.kind'), kc.subagent_kind) AS kind,
         coalesce(c.calls, 0) AS calls, c.last_call_at,
         coalesce(st.turn_id, fc.turn_id) AS turn_id
    FROM ids i
    LEFT JOIN ev_firsts f ON f.agent_session_id = i.agent_session_id AND f.id = i.id
    LEFT JOIN calls c ON c.agent_session_id = i.agent_session_id AND c.id = i.id
    LEFT JOIN source('event_log') st ON st.seq = f.start_seq
    LEFT JOIN source('event_log') fin ON fin.seq = f.finish_seq
    LEFT JOIN source('event_log') k ON k.seq = f.kind_seq
    LEFT JOIN source('agent_tool_call') fc ON fc.id = c.first_call
    LEFT JOIN source('agent_tool_call') kc ON kc.id = c.kind_call
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
       CAST(uri('oxplow/agent_session', 'id', 'ses' || j.agent_session_id) AS TEXT) AS session_uri,
       CAST(uri('oxplow/subagent', 'session', 'ses' || j.agent_session_id, 'agent', j.id) AS TEXT) AS uri
FROM joined j
JOIN source('agent_session') s ON s.id = j.agent_session_id
