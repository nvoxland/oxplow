-- Archived work leaves search (tsk921): a comment in an archived stream
-- isn't indexed.
SELECT c.uri AS uri,
       CAST(c.quote AS TEXT) AS title,
       CAST(c.quote || coalesce(
         (SELECT group_concat(char(10) || m.body, '')
            FROM (SELECT body FROM source('comment_message')
                   WHERE comment_id = c.id ORDER BY id) m),
         '') AS TEXT) AS body,
       CAST('str' || c.stream_id AS TEXT) AS stream_id,
       -- It changes when a message is added (which bumps `last_activity_at`)
       -- or the comment itself is edited.
       CAST(max(c.updated_at, c.last_activity_at) AS TEXT) AS version
FROM ref('comment') c
LEFT JOIN source('streams') s ON s.id = c.stream_id
WHERE s.archived_at IS NULL
