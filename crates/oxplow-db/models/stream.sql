SELECT s.id, s.kind, s.title, s.branch, s.worktree_path, s.created_at, s.updated_at, s.archived_at,
       CAST(uri('oxplow/stream', 'id', 'str' || s.id) AS TEXT) AS ref,
       s.custom_prompt AS prompt,
       h.head_sha
FROM source('streams') s
LEFT JOIN source('stream_head') h ON h.stream_id = s.id
