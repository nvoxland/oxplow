SELECT t.id, t.thread_id, th.stream_id, t.parent_id, t.title, t.description,
       t.status, t.priority, t.author, t.sort_index, t.created_at, t.updated_at,
       t.completed_at
FROM source('task') t
LEFT JOIN source('threads') th ON th.id = t.thread_id
WHERE t.deleted_at IS NULL
