SELECT e.id AS event_id, e.stream_id,
       CAST(json_extract(CAST(c.bytes AS TEXT), '$.stderr') AS TEXT) AS stderr,
       CAST(json_extract(CAST(c.bytes AS TEXT), '$.stdout') AS TEXT) AS stdout,
       CAST(coalesce(json_extract(e.payload, '$.output.truncated'), 0) AS INTEGER) AS truncated
FROM ref('event') e
JOIN source('event_content') c ON c.hash = json_extract(e.payload, '$.output.hash')
WHERE e.type = 'ui.op_failed'
