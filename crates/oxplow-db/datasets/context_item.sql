SELECT c.id, c.thread_id,
       (SELECT t.uri FROM ref('thread') t WHERE t.id = c.thread_id) AS thread_uri,
       c.basket_id, b.name AS basket, c.stream_id,
       c.kind, c.source_uri, c.label, c.text, c.around, c.note, c.added_at, c.held_at
FROM source('context_item') c
LEFT JOIN source('context_basket') b ON b.id = c.basket_id
