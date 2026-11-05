-- From each open session to its own newest status change: the log only grows,
-- so this must not walk every status change ever logged.
SELECT e.thread_id,
       e.agent_session_id,
       json_extract(e.payload, '$.state') AS state,
       json_extract(e.payload, '$.detail') AS detail,
       e.at AS since
FROM source('agent_session') s
JOIN source('event_log') e
  ON e.seq = (SELECT x.seq FROM source('event_log') x
               WHERE x.agent_session_id = s.id
                 AND x.type = 'agent.status.changed'
               ORDER BY x.seq DESC LIMIT 1)
WHERE s.closed_at IS NULL
