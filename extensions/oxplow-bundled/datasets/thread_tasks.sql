-- Each thread's Work panel, one row per line in display order: the
-- tasks the thread is working on, what is next, and what it finished — a
-- project-manager view of tasks, never of turns or requests.
-- In progress is the tasks assigned to the thread in progress (under their
-- epic, with the epic's other children, when they have one). Up next is the
-- tasks assigned to the thread that are ready or blocked, in its queue's
-- order. It reads no request: what a thread's request is doing now changes
-- on every tool call, and a reader that shows it (the Work panel's `now`)
-- reads that for its own thread, so this list re-runs only when tasks do.
-- Finished is the done tasks the thread was working (v_request_task, basis
-- assigned or transition: it held the task or moved it; filing, commenting
-- or being prompted with its id is not working it) since the person last
-- cleared the list, and the knowledge pages it touched. An epic is a task with
-- children; it's shown as context, never as up next.
-- The tasks that have children, found once: an EXISTS per task searched
-- every task for each.
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
doing AS (
  SELECT i.* FROM item i
  WHERE i.thread_id IS NOT NULL AND i.state = 'in_progress' AND NOT i.is_epic
),
epic AS (
  SELECT d.thread_id, e.uri AS epic_ref, e.title AS epic_title, min(d.pos) AS first_pos
  FROM doing d JOIN item e ON e.uri = d.parent_uri AND e.is_epic
  GROUP BY d.thread_id, e.uri, e.title
),
finished AS (
  SELECT thread_id, title, icon, uri, at FROM (
    SELECT rt.thread_id, i.title, 'done' AS icon, i.uri, i.closed_at AS at,
           row_number() OVER (PARTITION BY rt.thread_id, i.uri ORDER BY i.closed_at DESC) AS n
    FROM item i
    JOIN ref('request_task') rt ON rt.task = i.uri
    WHERE i.state = 'done' AND rt.thread_id IS NOT NULL AND rt.basis IN ('assigned', 'transition')
  ) WHERE n = 1
  UNION ALL
  SELECT k.thread_id, p.title, 'knowledge', p.uri, k.last_seen_at
  FROM ref('knowledge_touch') k JOIN ref('knowledge_page') p ON p.uri = k.page
),
-- s1..s3 order the In progress block: each epic (s1 = 0) with its
-- children right after its header, then the stand-alone tasks (s1 = 1).
lines AS (
SELECT d.thread_id, 'In progress' AS grp, 0 AS g, d.pos AS ord, 1 AS s1, 0 AS s2, 0 AS s3, d.title,
       d.state AS icon, d.uri, 0 AS depth, 1 AS active
FROM doing d
WHERE NOT EXISTS (SELECT 1 FROM epic e WHERE e.thread_id = d.thread_id AND e.epic_ref = d.parent_uri)
UNION ALL
SELECT e.thread_id, 'In progress', 0, 0, 0, e.first_pos, 0, e.epic_title, 'epic', e.epic_ref, 0, 0
FROM epic e
UNION ALL
-- A child in progress on another thread is shown but isn't this thread's
-- active task.
SELECT e.thread_id, 'In progress', 0, 0, 0, e.first_pos,
       CASE WHEN c.thread_id = e.thread_id THEN 0 ELSE 1000000 END + c.pos, c.title, c.state, c.uri, 1,
       c.state = 'in_progress' AND c.thread_id = e.thread_id
FROM epic e JOIN item c ON c.parent_uri = e.epic_ref AND c.state <> 'canceled'
UNION ALL
SELECT thread_id, 'Up next', 1, rn, 0, 0, 0, title, state, uri, 0, 0 FROM (
  SELECT i.*, row_number() OVER (PARTITION BY i.thread_id ORDER BY i.pos) AS rn
  FROM item i
  WHERE i.thread_id IS NOT NULL AND i.state IN ('todo', 'blocked') AND NOT i.is_epic
)
UNION ALL
SELECT thread_id, 'Finished', 3, rn, 0, 0, 0, title, icon, uri, 0, 0 FROM (
  SELECT f.*, row_number() OVER (PARTITION BY f.thread_id ORDER BY f.at DESC) AS rn
  FROM finished f LEFT JOIN ref('finished_cleared') c ON c.thread_id = f.thread_id
  WHERE c.cleared_at IS NULL OR f.at > c.cleared_at
) WHERE rn <= 20
)
-- One type per column: a UNION blends its arms' types.
SELECT CAST(thread_id AS INTEGER) AS thread_id, CAST(grp AS TEXT) AS grp, CAST(g AS INTEGER) AS g,
       CAST(row_number() OVER (PARTITION BY thread_id, g ORDER BY s1, s2, s3, ord) AS INTEGER) AS ord,
       CAST(title AS TEXT) AS title, CAST(icon AS TEXT) AS icon,
       CAST(uri AS TEXT) AS uri, CAST(depth AS INTEGER) AS depth, CAST(active AS INTEGER) AS active
FROM lines
