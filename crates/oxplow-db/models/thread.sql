SELECT id, stream_id, title, sort_index, created_at, updated_at,
       closed_at, archived_at,
       CAST('thread:thr' || id AS TEXT) AS ref,
       mark
FROM source('threads')
