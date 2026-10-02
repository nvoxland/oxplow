SELECT owner, id, status, last_run_at, error, row_counts_json, cursor_json, last_event_id
FROM source('collector_run')
