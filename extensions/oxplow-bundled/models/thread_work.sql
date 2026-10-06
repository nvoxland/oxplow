-- Each thread's Work panel, one row per line in display order: what's in
-- progress — the task (under its epic, with the epic's other children,
-- when it has one) and an open effort no item names — the next ready
-- tasks, items on an outside tracker, and what finished since the person
-- last cleared the list (tasks, wiki pages, unlinked efforts). An epic is
-- a task with children; it's shown as context, never as the active item.
-- A linked effort shows as its item.
WITH task AS (
  SELECT t.*, EXISTS (SELECT 1 FROM ref('task') c WHERE c.parent_id = t.id) AS is_epic
  FROM ref('task') t
),
active AS (
  SELECT * FROM (
    SELECT t.*, row_number() OVER (PARTITION BY t.thread_id ORDER BY t.sort_index, t.created_at) AS rn
    FROM task t
    WHERE t.status = 'in_progress' AND NOT t.is_epic AND t.thread_id IS NOT NULL
  ) WHERE rn = 1
),
epic AS (
  SELECT a.thread_id, e.id AS epic_id, e.title AS epic_title, a.id AS active_id
  FROM active a JOIN task e ON e.id = a.parent_id AND e.is_epic
),
unlinked AS (
  SELECT e.thread_id, e.id, coalesce(e.title, 'Work in progress') AS title, e.ended_at
  FROM ref('effort') e
  WHERE e.work_item IS NULL
),
finished AS (
  SELECT t.thread_id, t.title, 'done' AS icon, 'work_item:oxplow:tsk' || t.id AS ref, t.completed_at AS at
  FROM task t
  WHERE t.status = 'done' AND t.completed_at IS NOT NULL AND t.thread_id IS NOT NULL
  UNION ALL
  SELECT k.thread_id, p.title, 'wiki', 'wiki:' || p.slug, k.last_seen_at
  FROM ref('knowledge_touch') k JOIN ref('knowledge_page') p ON p.ref = k.page
  UNION ALL
  SELECT u.thread_id, u.title, 'done', 'effort:eff' || u.id, u.ended_at
  FROM unlinked u WHERE u.ended_at IS NOT NULL
),
lines AS (
SELECT a.thread_id, 'In progress' AS grp, 0 AS g, 0 AS ord, a.title, a.status AS icon,
       'work_item:oxplow:tsk' || a.id AS ref, 0 AS depth, 1 AS active
FROM active a
WHERE NOT EXISTS (SELECT 1 FROM epic e WHERE e.thread_id = a.thread_id)
UNION ALL
SELECT u.thread_id, 'In progress', 0, -2, u.title, 'in_progress', 'effort:eff' || u.id, 0, 1
FROM unlinked u WHERE u.ended_at IS NULL
UNION ALL
SELECT e.thread_id, 'In progress', 0, -1, e.epic_title, 'epic', 'work_item:oxplow:tsk' || e.epic_id, 0, 0
FROM epic e
UNION ALL
SELECT e.thread_id, 'In progress', 0, c.sort_index, c.title, c.status, 'work_item:oxplow:tsk' || c.id, 1,
       c.id = e.active_id
FROM epic e JOIN task c ON c.parent_id = e.epic_id AND c.status <> 'archived'
UNION ALL
SELECT thread_id, 'Ready', 1, rn, title, 'ready', 'work_item:oxplow:tsk' || id, 0, 0 FROM (
  SELECT t.*, row_number() OVER (PARTITION BY t.thread_id ORDER BY t.sort_index, t.created_at) AS rn
  FROM task t
  WHERE t.status = 'ready' AND NOT t.is_epic AND t.thread_id IS NOT NULL
) WHERE rn <= 10
UNION ALL
SELECT thread_id, 'On your tracker', 2, rn, title || ' · ' || provider, state, ref, 0, 0 FROM (
  SELECT w.*, row_number() OVER (PARTITION BY w.thread_id ORDER BY w.created_at) AS rn
  FROM ref('work_item') w
  WHERE w.provider <> 'oxplow' AND w.state IN ('todo', 'in_progress', 'blocked') AND w.thread_id IS NOT NULL
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
