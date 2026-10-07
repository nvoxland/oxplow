-- Each thread's Work panel, one row per line in display order: what's in
-- progress — the item (under its epic, with the epic's other children,
-- when it has one) and an open effort no item names — the next ready
-- items, and what finished since the person last cleared the list (items,
-- wiki pages, unlinked efforts). Read from the work-item interface, so
-- it's whichever list is active. An epic is an item with children; it's
-- shown as context, never as the active item. A linked effort shows as
-- its item.
WITH item AS (
  SELECT w.*, EXISTS (SELECT 1 FROM ref('work_item') c WHERE c.parent_ref = w.ref) AS is_epic,
         row_number() OVER (PARTITION BY w.thread_id
                            ORDER BY w.rank IS NULL, w.rank, w.created_at) AS pos
  FROM ref('work_item') w
),
active AS (
  SELECT * FROM (
    SELECT i.*, row_number() OVER (PARTITION BY i.thread_id ORDER BY i.pos) AS rn
    FROM item i
    WHERE i.state = 'in_progress' AND NOT i.is_epic AND i.thread_id IS NOT NULL
  ) WHERE rn = 1
),
epic AS (
  SELECT a.thread_id, e.ref AS epic_ref, e.title AS epic_title, a.ref AS active_ref
  FROM active a JOIN item e ON e.ref = a.parent_ref AND e.is_epic
),
unlinked AS (
  SELECT e.thread_id, e.id, coalesce(e.title, 'Work in progress') AS title, e.ended_at
  FROM ref('effort') e
  WHERE e.work_item IS NULL
),
finished AS (
  SELECT i.thread_id, i.title, 'done' AS icon, i.ref, i.closed_at AS at
  FROM item i
  WHERE i.state = 'done' AND i.closed_at IS NOT NULL AND i.thread_id IS NOT NULL
  UNION ALL
  SELECT k.thread_id, p.title, 'wiki', 'wiki:' || p.slug, k.last_seen_at
  FROM ref('knowledge_touch') k JOIN ref('knowledge_page') p ON p.ref = k.page
  UNION ALL
  SELECT u.thread_id, u.title, 'done', 'effort:eff' || u.id, u.ended_at
  FROM unlinked u WHERE u.ended_at IS NOT NULL
),
lines AS (
SELECT a.thread_id, 'In progress' AS grp, 0 AS g, 0 AS ord, a.title, a.state AS icon,
       a.ref, 0 AS depth, 1 AS active
FROM active a
WHERE NOT EXISTS (SELECT 1 FROM epic e WHERE e.thread_id = a.thread_id)
UNION ALL
SELECT u.thread_id, 'In progress', 0, -2, u.title, 'in_progress', 'effort:eff' || u.id, 0, 1
FROM unlinked u WHERE u.ended_at IS NULL
UNION ALL
SELECT e.thread_id, 'In progress', 0, -1, e.epic_title, 'epic', e.epic_ref, 0, 0
FROM epic e
UNION ALL
SELECT e.thread_id, 'In progress', 0, c.pos, c.title, c.state, c.ref, 1, c.ref = e.active_ref
FROM epic e JOIN item c ON c.parent_ref = e.epic_ref AND c.state <> 'canceled'
UNION ALL
SELECT thread_id, 'Ready', 1, rn, title, 'todo', ref, 0, 0 FROM (
  SELECT i.*, row_number() OVER (PARTITION BY i.thread_id ORDER BY i.pos) AS rn
  FROM item i
  WHERE i.state = 'todo' AND NOT i.is_epic AND i.thread_id IS NOT NULL
) WHERE rn <= 10
UNION ALL
SELECT thread_id, 'Finished', 3, rn, title, icon, ref, 0, 0 FROM (
  SELECT f.*, row_number() OVER (PARTITION BY f.thread_id ORDER BY f.at DESC) AS rn
  FROM finished f LEFT JOIN ref('finished_cleared') c ON c.thread_id = f.thread_id
  WHERE c.cleared_at IS NULL OR f.at > c.cleared_at
) WHERE rn <= 5
)
-- One type per column: a UNION blends its arms' types.
SELECT CAST(thread_id AS INTEGER) AS thread_id, CAST(grp AS TEXT) AS grp, CAST(g AS INTEGER) AS g,
       CAST(ord AS INTEGER) AS ord, CAST(title AS TEXT) AS title, CAST(icon AS TEXT) AS icon,
       CAST(ref AS TEXT) AS ref, CAST(depth AS INTEGER) AS depth, CAST(active AS INTEGER) AS active
FROM lines
