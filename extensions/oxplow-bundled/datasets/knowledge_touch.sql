-- Which threads wrote which knowledge pages, and when last: the
-- `knowledge_touch_record` model, written when a page changes by a command.
SELECT CAST(m.page AS TEXT) AS page,
       CAST(th.id AS INT) AS thread_id,
       CAST(m.last_seen AS TEXT) AS last_seen_at
FROM ref('knowledge_touch_record') m
JOIN ref('thread') th ON th.uri = m.thread
