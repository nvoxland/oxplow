SELECT s.thread_id,
       (SELECT t.uri FROM ref('thread') t WHERE t.id = s.thread_id) AS thread_uri,
       s.basket_name AS basket,
       CAST(count(*) AS INTEGER) AS items,
       CAST(max(s.sent_at) AS TEXT) AS sent_at
FROM source('context_sent_item') s
GROUP BY s.thread_id, s.basket_name
