SELECT id, stream_id, title, sort_index, created_at, updated_at,
       closed_at, archived_at,
       CAST('thread:thr' || id AS TEXT) AS ref,
       mark,
       custom_prompt AS prompt
FROM source('threads')
