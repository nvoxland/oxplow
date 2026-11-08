SELECT e.id AS event_id, e.at, e.stream_id,
       CAST(json_extract(e.payload, '$.thread') AS TEXT) AS thread,
       CAST(json_extract(e.payload, '$.label') AS TEXT) AS label,
       CAST(json_extract(e.payload, '$.command') AS TEXT) AS command,
       CAST(json_extract(e.payload, '$.message') AS TEXT) AS message,
       CAST(json_extract(e.payload, '$.exit_code') AS INTEGER) AS exit_code,
       CAST(json_extract(e.payload, '$.signal') AS TEXT) AS signal,
       CAST(json_extract(e.payload, '$.duration_ms') AS INTEGER) AS duration_ms,
       CAST(json_extract(e.payload, '$.output.size') AS INTEGER) AS output_size,
       CAST(json_extract(e.payload, '$.audit_id') AS INTEGER) AS audit_id,
       CAST(json_extract(e.payload, '$.proposal') AS INTEGER) AS proposal_id,
       e.payload_expired_at,
       -- Dismissing it is seeing it.
       CAST(EXISTS (SELECT 1 FROM ref('event') m
                     WHERE m.type IN ('ui.op_error.seen', 'ui.op_error.dismissed')
                       AND json_extract(m.payload, '$.event') = e.id) AS INTEGER) AS seen,
       CAST(EXISTS (SELECT 1 FROM ref('event') m
                     WHERE m.type = 'ui.op_error.dismissed'
                       AND json_extract(m.payload, '$.event') = e.id) AS INTEGER) AS dismissed
FROM ref('event') e
WHERE e.type = 'ui.op_failed'
