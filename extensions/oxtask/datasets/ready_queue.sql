-- What to pick up next, one definition for the next-task page and the
-- agent's task state. Ready means `todo` with no open blocker. Per thread
-- (`thread_id`): its assigned ready items in its queue's order; when the
-- first is an epic (an item with children among them), it and its ready
-- descendants as one unit, else every ready one that isn't an epic. The
-- pool (`thread_id` NULL): ready items assigned to nobody that aren't
-- epics, by priority.
WITH held AS (
  SELECT l.to_uri FROM ref('oxplow/task_link') l JOIN ref('oxplow/task') b ON b.uri = l.from_uri
  WHERE l.link_type = 'blocks' AND b.state NOT IN ('done', 'canceled')),
items AS (
  SELECT t.id AS thread_id, w.uri, w.title, w.native_id, w.state, w.parent_uri, w.rank, w.created_at
  FROM ref('oxplow/task') w JOIN ref('thread') t ON t.uri = w.assignee),
epics AS (SELECT DISTINCT thread_id, parent_uri AS uri FROM items WHERE parent_uri IS NOT NULL),
ready AS (
  SELECT * FROM items WHERE state = 'todo' AND uri NOT IN (SELECT to_uri FROM held)),
head AS (
  SELECT thread_id, uri FROM (
    SELECT thread_id, uri,
           row_number() OVER (PARTITION BY thread_id ORDER BY rank IS NULL, rank, created_at) AS n
    FROM ready)
  WHERE n = 1),
below(thread_id, uri) AS (
  SELECT i.thread_id, i.uri FROM items i JOIN head h ON i.thread_id = h.thread_id AND i.parent_uri = h.uri
  UNION SELECT i.thread_id, i.uri FROM items i JOIN below b ON i.thread_id = b.thread_id AND i.parent_uri = b.uri),
mine AS (
  SELECT r.thread_id, r.uri, r.title, r.native_id,
         CASE WHEN r.uri = h.uri AND EXISTS (SELECT 1 FROM epics e WHERE e.thread_id = r.thread_id AND e.uri = r.uri) THEN 'epic'
              WHEN EXISTS (SELECT 1 FROM below b WHERE b.thread_id = r.thread_id AND b.uri = r.uri) THEN 'child'
              ELSE 'ready' END AS role,
         r.rank, r.created_at
  FROM ready r JOIN head h ON h.thread_id = r.thread_id
  WHERE CASE WHEN EXISTS (SELECT 1 FROM epics e WHERE e.thread_id = h.thread_id AND e.uri = h.uri)
             THEN r.uri = h.uri OR EXISTS (SELECT 1 FROM below b WHERE b.thread_id = r.thread_id AND b.uri = r.uri)
             ELSE NOT EXISTS (SELECT 1 FROM epics e WHERE e.thread_id = r.thread_id AND e.uri = r.uri) END),
pool AS (
  SELECT p.uri, p.title, p.native_id, p.rank, p.created_at,
         CASE json_extract(CASE WHEN json_valid(p.native) THEN p.native END, '$.priority')
           WHEN 'urgent' THEN 0 WHEN 'high' THEN 1 WHEN 'low' THEN 3 ELSE 2 END AS urgency
  FROM ref('oxplow/task') p
  WHERE p.assignee IS NULL AND p.state = 'todo'
    AND p.uri NOT IN (SELECT to_uri FROM held)
    AND p.uri NOT IN (SELECT parent_uri FROM ref('oxplow/task') WHERE parent_uri IS NOT NULL))
SELECT thread_id, uri, title, native_id, CAST(role AS TEXT) AS role,
       CAST(row_number() OVER (PARTITION BY thread_id ORDER BY role = 'child', rank IS NULL, rank, created_at) AS INTEGER) AS ord
FROM mine
UNION ALL
SELECT NULL, uri, title, native_id, CAST('pool' AS TEXT),
       CAST(row_number() OVER (ORDER BY urgency, rank IS NULL, rank, created_at) AS INTEGER)
FROM pool
