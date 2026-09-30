SELECT snapshot_id, stream_id, files_collected, files_over_budget,
       files_without_server, files_failed, captured_at
FROM source('symbol_capture')
