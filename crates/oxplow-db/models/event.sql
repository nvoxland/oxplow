SELECT seq, id, type, v, at, source,
       stream_id, thread_id, effort_id, turn_id, snapshot_id,
       subject, payload, payload_hash, payload_expired_at, cause, dedupe_key
FROM source('event_log')
