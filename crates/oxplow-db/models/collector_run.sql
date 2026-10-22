SELECT owner, id, status, last_run_at, error, row_counts_json, cursor_json, last_event_id,
       run_id,
       CAST(uri('oxplow/collector', 'collector', owner || '/' || id) AS TEXT) AS uri
FROM source('collector_run')
