-- Where each review check stands for each analysed change. A check is a
-- collector on `change.analyzed`; a change's checks run after its analysis
-- is stored. Its state is read from the check's own run for that change: the
-- run the change's latest analysis event caused (`cause_event_id`), never the
-- collector's latest run, which may be another change's. `failed` when it
-- failed, `ok` when it finished otherwise, `pending` while it is live. With
-- no run: `off` when the collector is disabled now (a disabled collector
-- records nothing); `pending` while the collector has not handled the event
-- yet (its checkpoint's `last_event_id`, an event seq, is before the
-- change's); else `not_run`: it handled a later event and never ran for this
-- one (it was disabled then, or retention swept the run), so it never will.
WITH checks AS (
  SELECT c.uri AS collector_uri, c.namespace || '/' || c.local AS collector,
         c.namespace, c.local
    FROM ref('contribution') c, json_each(json_extract(c.runs_on, '$.types')) t
   WHERE c.kind = 'collector' AND json_extract(c.runs_on, '$.on') = 'events'
     AND t.value = 'change.analyzed'),
latest AS (
  SELECT c.id AS change_id, e.id AS event_id, c.files_analyzed_seq AS event_seq
    FROM ref('change') c JOIN ref('event') e ON e.seq = c.files_analyzed_seq
   WHERE c.files_analyzed_seq IS NOT NULL)
SELECT CAST(l.change_id AS INTEGER) AS change_id, CAST(k.collector AS TEXT) AS collector,
       CAST(k.collector_uri AS TEXT) AS collector_uri,
       CAST(CASE WHEN r.run_id IS NULL AND h.state = 'disabled' THEN 'off'
                 WHEN r.run_id IS NULL AND coalesce(p.last_event_id, 0) >= l.event_seq THEN 'not_run'
                 WHEN r.run_id IS NULL OR r.outcome IS NULL THEN 'pending'
                 WHEN r.outcome IN ('failed', 'denied', 'cancelled', 'interrupted', 'invalid') THEN 'failed'
                 ELSE 'ok' END AS TEXT) AS state,
       CAST(r.finished_at AS TEXT) AS last_run_at,
       CAST(CASE WHEN r.outcome IN ('failed', 'denied', 'cancelled', 'interrupted', 'invalid')
                 THEN r.error END AS TEXT) AS error
  FROM latest l CROSS JOIN checks k
  LEFT JOIN ref('contribution_health') h ON h.uri = k.collector_uri
  LEFT JOIN ref('collector_checkpoint') p ON p.owner = k.namespace AND p.id = k.local
  LEFT JOIN ref('contribution_run') r
    ON r.run_id = (SELECT max(x.run_id) FROM ref('contribution_run') x
                    WHERE x.contribution = k.collector_uri AND x.cause_event_id = l.event_id)
