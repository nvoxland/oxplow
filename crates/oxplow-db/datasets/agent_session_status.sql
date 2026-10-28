SELECT e.thread_id,
       e.agent_session_id,
       json_extract(e.payload, '$.state') AS state,
       json_extract(e.payload, '$.detail') AS detail,
       e.at AS since
FROM source('event_log') e
JOIN source('agent_session') s ON s.id = e.agent_session_id
WHERE e.type = 'agent.status.changed'
  AND s.closed_at IS NULL
  AND e.seq = (SELECT max(x.seq) FROM source('event_log') x
                WHERE x.type = 'agent.status.changed'
                  AND x.agent_session_id = e.agent_session_id)
