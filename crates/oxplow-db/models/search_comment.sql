SELECT CAST('comment:cmt' || c.id AS TEXT) AS ref,
       CAST(c.quote AS TEXT) AS title,
       CAST(c.quote || coalesce(
         (SELECT group_concat(char(10) || m.body, '')
            FROM (SELECT body FROM source('comment_message')
                   WHERE comment_id = c.id ORDER BY id) m),
         '') AS TEXT) AS body,
       CAST('str' || c.stream_id AS TEXT) AS stream_id
FROM source('comment') c
