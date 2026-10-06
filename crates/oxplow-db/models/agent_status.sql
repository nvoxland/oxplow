SELECT e.thread_id,
       json_extract(e.payload, '$.state') AS state,
       json_extract(e.payload, '$.detail') AS detail,
       e.at AS since
FROM source('event_log') e
WHERE e.type = 'agent.status.changed'
  AND e.thread_id IS NOT NULL
  AND e.seq = (SELECT max(s.seq) FROM source('event_log') s
                WHERE s.type = 'agent.status.changed' AND s.thread_id = e.thread_id)
