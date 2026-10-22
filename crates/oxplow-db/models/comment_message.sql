SELECT CAST(uri('oxplow/comment', 'id', 'cmt' || m.comment_id) AS TEXT) AS comment,
       (SELECT count(*) FROM source('comment_message') e
         WHERE e.comment_id = m.comment_id AND e.id <= m.id) AS position,
       m.author, m.body, m.created_at
FROM source('comment_message') m
