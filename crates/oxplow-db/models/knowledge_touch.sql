SELECT CAST('wiki:' || u.slug AS TEXT) AS page, u.thread_id, u.last_seen_at
FROM source('wiki_page_thread_update') u
