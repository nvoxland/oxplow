SELECT id, stream_id, kind, target, base_revision, head_revision, status, error, computed_at,
       snapshot_id, events_to, files_at, files_events_to, conflicted, in_progress
FROM source('change')
