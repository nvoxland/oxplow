SELECT n.id, n.thread_id, n.body, n.author, n.created_at,
       CAST('thread_note:not' || n.id AS TEXT) AS ref,
       (SELECT t.ref FROM ref('thread') t WHERE t.id = n.thread_id) AS thread_ref
FROM source('thread_note') n
