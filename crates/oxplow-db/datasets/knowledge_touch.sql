SELECT p.uri AS page, t.thread_id, t.last_seen_at
FROM source('knowledge_page_touch') t
JOIN ref('knowledge_page') p ON p.uri = t.uri
