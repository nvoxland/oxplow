SELECT uri, snapshot_id, stream_id, path, name, kind, container, signature, language,
       line, col, start_line, start_col, end_line, end_col
FROM source('symbol')
