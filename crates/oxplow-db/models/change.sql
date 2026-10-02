SELECT id, stream_id, kind, target, base_revision, head_revision, status, error, computed_at,
       snapshot_id, events_to
FROM source('change')
