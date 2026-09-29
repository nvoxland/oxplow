SELECT id, stream_id, kind, target, base_label, head_label, status, error, computed_at
FROM source('change')
