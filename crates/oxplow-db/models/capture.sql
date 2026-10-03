SELECT id, stream_id, thread_id, effort_id, producer, status, trigger,
       provenance, source, snapshot_id, branch, closest_vcs_rev,
       captured_at, ended_at, scan_kind, turn_id
FROM source('metric_capture')
