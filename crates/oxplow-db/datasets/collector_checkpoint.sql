SELECT owner, id,
       CAST(uri('oxplow/collector', 'collector', owner || '/' || id) AS TEXT) AS uri,
       cursor_json, last_event_id, last_run_id, row_counts_json
FROM source('collector_checkpoint')
