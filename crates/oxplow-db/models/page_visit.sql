SELECT id, thread_id, page_kind, page_id, label, visited_at, duration_ms
FROM source('page_visit')
