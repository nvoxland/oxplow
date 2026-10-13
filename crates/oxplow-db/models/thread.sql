SELECT id, stream_id, title, status, sort_index, created_at, updated_at,
       closed_at, archived_at,
       CAST('thread:thr' || id AS TEXT) AS ref
FROM source('threads')
