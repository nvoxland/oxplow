SELECT c.id, c.stream_id, c.thread_id, c.target_resource, c.target_uri AS target, c.quote,
       c.intent, c.status, c.orphaned, c.author, c.created_at, c.updated_at,
       c.last_activity_at, c.resolved_at,
       (SELECT m.body FROM source('comment_message') m WHERE m.comment_id = c.id
         ORDER BY m.id LIMIT 1) AS body,
       (SELECT count(*) FROM source('comment_message') m WHERE m.comment_id = c.id)
         AS message_count,
       CAST(uri('oxplow/comment', 'id', 'cmt' || c.id) AS TEXT) AS uri
FROM source('comment') c
