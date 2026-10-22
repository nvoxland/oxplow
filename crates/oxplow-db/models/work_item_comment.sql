-- Comments on the active work list's items.
SELECT c.id, c.uri, c.body, c.author, c.created_at
FROM source('work_item_comment') c
JOIN ref('work_item') w ON w.uri = c.uri
