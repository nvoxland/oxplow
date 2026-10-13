SELECT owner, id, status, last_run_at, error, row_counts_json, cursor_json, last_event_id,
       run_id,
       CAST('collector:' || owner || '/' || id AS TEXT) AS ref
FROM source('collector_run')
