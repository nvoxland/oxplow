SELECT ref, snapshot_id, stream_id, path, name, kind, container, language,
       line, col, end_line, end_col
FROM source('symbol')
