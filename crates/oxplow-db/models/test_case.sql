SELECT c.id AS run_id,
       json_extract(s.value, '$.name') AS suite,
       json_extract(t.value, '$.classname') AS classname,
       json_extract(t.value, '$.name') AS name,
       json_extract(t.value, '$.status') AS status,
       json_extract(t.value, '$.timeMs') AS time_ms
FROM source('metric_capture') c,
     json_each(c.detail_json, '$.payload.suites') s,
     json_each(s.value, '$.cases') t
WHERE c.producer IN ('tests', 'test-run')
  AND json_extract(c.detail_json, '$.kind') = 'test-detail'
