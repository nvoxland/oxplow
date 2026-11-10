-- Comments on the active task tracker's items.
SELECT c.id, c.uri, c.body, c.author, c.created_at
FROM source('task_comment') c
JOIN ref('task') w ON w.uri = c.uri
