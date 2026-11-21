-- Where each review check stands for each analysed change. A check is a
-- collector on `change.analyzed`; a change's checks run after its analysis
-- is stored, so until a check has run for the change's latest analysis it is
-- `pending`, and `failed` when that run failed.
WITH checks AS (
  SELECT c.uri AS collector_uri, c.namespace || '/' || c.local AS collector
    FROM ref('contribution') c, json_each(json_extract(c.runs_on, '$.types')) t
   WHERE c.kind = 'collector' AND json_extract(c.runs_on, '$.on') = 'events'
     AND t.value = 'change.analyzed'),
latest AS (
  SELECT CAST(json_extract(e.payload, '$.change_id') AS INTEGER) AS change_id, max(e.seq) AS seq
    FROM ref('event') e
   WHERE e.type = 'change.analyzed' AND json_extract(e.payload, '$.phase') = 'files'
   GROUP BY 1)
SELECT CAST(l.change_id AS INTEGER) AS change_id, CAST(k.collector AS TEXT) AS collector,
       CAST(k.collector_uri AS TEXT) AS collector_uri,
       CAST(CASE WHEN r.last_event_id IS NULL OR r.last_event_id < l.seq THEN 'pending'
                 WHEN r.status = 'error' THEN 'failed'
                 ELSE 'ok' END AS TEXT) AS state,
       CAST(r.last_run_at AS TEXT) AS last_run_at,
       CAST(CASE WHEN r.status = 'error' THEN r.error END AS TEXT) AS error
  FROM latest l CROSS JOIN checks k
  LEFT JOIN ref('collector_run') r ON r.uri = k.collector_uri
