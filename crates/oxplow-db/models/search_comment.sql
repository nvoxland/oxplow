-- Archived work leaves search (tsk921): a comment in an archived stream
-- isn't indexed.
SELECT CAST('comment:cmt' || c.id AS TEXT) AS ref,
       CAST(c.quote AS TEXT) AS title,
       CAST(c.quote || coalesce(
         (SELECT group_concat(char(10) || m.body, '')
            FROM (SELECT body FROM source('comment_message')
                   WHERE comment_id = c.id ORDER BY id) m),
         '') AS TEXT) AS body,
       CAST('str' || c.stream_id AS TEXT) AS stream_id
FROM source('comment') c
LEFT JOIN source('streams') s ON s.id = c.stream_id
WHERE s.archived_at IS NULL
