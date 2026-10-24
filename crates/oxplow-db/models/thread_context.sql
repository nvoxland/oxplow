SELECT c.id, c.thread_id, c.kind, c.source_uri, c.label, c.text, c.added_at,
       c.delivered_at, c.delivered_session_id,
       (SELECT t.uri FROM ref('thread') t WHERE t.id = c.thread_id) AS thread_uri
FROM source('thread_context_item') c
