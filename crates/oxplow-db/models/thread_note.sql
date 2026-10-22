SELECT n.id, n.thread_id, n.body, n.author, n.created_at,
       CAST(uri('oxplow/thread_note', 'id', 'not' || n.id) AS TEXT) AS uri,
       (SELECT t.uri FROM ref('thread') t WHERE t.id = n.thread_id) AS thread_uri
FROM source('thread_note') n
