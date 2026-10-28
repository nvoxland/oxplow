-- Each of oxplow's tasks as an `e2e/item` URI (`oxplow:e2e/item?id=<n>`, the
-- task's own number).
SELECT CAST(uri('e2e/item', 'id', substr(w.native_id, 4)) AS TEXT) AS uri,
       CAST(w.title AS TEXT) AS title
FROM ref('work_item') w
WHERE w.provider = 'oxplow'
