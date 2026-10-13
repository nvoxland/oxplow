SELECT id, thread_id, body, author, created_at,
       CAST('thread_note:not' || id AS TEXT) AS ref
FROM source('thread_note')
