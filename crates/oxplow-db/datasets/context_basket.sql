SELECT b.id, b.name, b.attached_thread_id,
       (SELECT t.uri FROM ref('thread') t WHERE t.id = b.attached_thread_id) AS attached_thread_uri,
       CAST((SELECT count(*) FROM source('context_item') c WHERE c.basket_id = b.id) AS INTEGER) AS items,
       b.created_at
FROM source('context_basket') b
