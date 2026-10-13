SELECT p.ref AS page, u.thread_id, u.last_seen_at
FROM source('wiki_page_thread_update') u
JOIN ref('knowledge_page') p ON p.slug = u.slug
