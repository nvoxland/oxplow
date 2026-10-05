SELECT id AS event_id, at, stream_id,
       CAST(json_extract(payload, '$.thread') AS TEXT) AS thread,
       CAST(json_extract(payload, '$.label') AS TEXT) AS label,
       CAST(json_extract(payload, '$.command') AS TEXT) AS command,
       CAST(json_extract(payload, '$.message') AS TEXT) AS message,
       CAST(json_extract(payload, '$.exit_code') AS INTEGER) AS exit_code,
       CAST(json_extract(payload, '$.signal') AS TEXT) AS signal,
       CAST(json_extract(payload, '$.duration_ms') AS INTEGER) AS duration_ms,
       CAST(json_extract(payload, '$.output.size') AS INTEGER) AS output_size,
       payload_expired_at
FROM ref('event')
WHERE type = 'ui.op_failed'
