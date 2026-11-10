-- Each thread's Work panel, one row per line in display order: what the
-- thread is working on, from its efforts — not a view of task states.
-- In progress is its open effort: the item it's linked to (under its
-- epic, with the epic's other children, when it has one), or the
-- effort's own title. Up next is the items assigned to the thread that
-- are ready or blocked, in its queue's order (what a person or a
-- controller agent gave it). Finished is its closed efforts since the
-- person last cleared the list, each as its item (once) or its title,
-- and the wiki pages it touched. An epic is an item with children; it's
-- shown as context, never as up next.
-- The items that have children, found once: an EXISTS per item searched
-- every item for each (1.2 s over 1,600 items, and `item` is read several
-- times below).
WITH parent AS (
  SELECT DISTINCT parent_uri AS uri FROM ref('task') WHERE parent_uri IS NOT NULL
),
item AS (
  SELECT w.*, p.uri IS NOT NULL AS is_epic, t.id AS thread_id,
         row_number() OVER (PARTITION BY w.assignee
                            ORDER BY w.rank IS NULL, w.rank, w.created_at, w.uri) AS pos
  FROM ref('task') w
  LEFT JOIN parent p ON p.uri = w.uri
  LEFT JOIN ref('thread') t ON t.uri = w.assignee
),
open_effort AS (
  SELECT e.thread_id, e.uri, e.title, i.uri AS item_ref, i.title AS item_title,
         i.state AS item_state, i.parent_uri
  FROM ref('effort') e LEFT JOIN item i ON i.uri = e.task
  WHERE e.ended_at IS NULL
),
epic AS (
  SELECT o.thread_id, e.uri AS epic_ref, e.title AS epic_title, o.item_ref AS active_ref
  FROM open_effort o JOIN item e ON e.uri = o.parent_uri AND e.is_epic
),
finished AS (
  SELECT thread_id, title, icon, uri, at FROM (
    SELECT e.thread_id, coalesce(i.title, e.title) AS title, 'done' AS icon,
           coalesce(i.uri, e.uri) AS uri, e.ended_at AS at,
           row_number() OVER (PARTITION BY e.thread_id, coalesce(i.uri, e.uri)
                              ORDER BY e.ended_at DESC) AS n
    FROM ref('effort') e LEFT JOIN item i ON i.uri = e.task
    WHERE e.ended_at IS NOT NULL
  ) WHERE n = 1
  UNION ALL
  SELECT k.thread_id, p.title, 'wiki', p.uri, k.last_seen_at
  FROM ref('knowledge_touch') k JOIN ref('knowledge_page') p ON p.uri = k.page
),
lines AS (
SELECT o.thread_id, 'In progress' AS grp, 0 AS g, 0 AS ord,
       coalesce(o.item_title, o.title) AS title,
       coalesce(o.item_state, 'in_progress') AS icon,
       coalesce(o.item_ref, o.uri) AS uri, 0 AS depth, 1 AS active
FROM open_effort o
WHERE NOT EXISTS (SELECT 1 FROM epic e WHERE e.thread_id = o.thread_id)
UNION ALL
SELECT e.thread_id, 'In progress', 0, -1, e.epic_title, 'epic', e.epic_ref, 0, 0
FROM epic e
UNION ALL
SELECT e.thread_id, 'In progress', 0, c.pos, c.title, c.state, c.uri, 1, c.uri = e.active_ref
FROM epic e JOIN item c ON c.parent_uri = e.epic_ref AND c.state <> 'canceled'
UNION ALL
SELECT thread_id, 'Up next', 1, rn, title, state, uri, 0, 0 FROM (
  SELECT i.*, row_number() OVER (PARTITION BY i.thread_id ORDER BY i.pos) AS rn
  FROM item i
  WHERE i.thread_id IS NOT NULL AND i.state IN ('todo', 'blocked') AND NOT i.is_epic
    AND NOT EXISTS (SELECT 1 FROM open_effort o WHERE o.item_ref = i.uri)
)
UNION ALL
SELECT thread_id, 'Finished', 3, rn, title, icon, uri, 0, 0 FROM (
  SELECT f.*, row_number() OVER (PARTITION BY f.thread_id ORDER BY f.at DESC) AS rn
  FROM finished f LEFT JOIN ref('finished_cleared') c ON c.thread_id = f.thread_id
  WHERE c.cleared_at IS NULL OR f.at > c.cleared_at
) WHERE rn <= 20
)
-- One type per column: a UNION blends its arms' types.
SELECT CAST(thread_id AS INTEGER) AS thread_id, CAST(grp AS TEXT) AS grp, CAST(g AS INTEGER) AS g,
       CAST(ord AS INTEGER) AS ord, CAST(title AS TEXT) AS title, CAST(icon AS TEXT) AS icon,
       CAST(uri AS TEXT) AS uri, CAST(depth AS INTEGER) AS depth, CAST(active AS INTEGER) AS active
FROM lines
