-- A badge for each thread that worked on tasks: how many are done and how
-- many are still open, counted over the tasks its requests worked on
-- (`request_task`). Decorates `oxplow/thread` rows (closed threads).
SELECT CAST(th.uri AS TEXT) AS uri,
       CAST(trim(
         CASE WHEN n.done > 0 THEN n.done || ' done' ELSE '' END
         || CASE WHEN n.done > 0 AND n.open > 0 THEN ' · ' ELSE '' END
         || CASE WHEN n.open > 0 THEN n.open || ' open' ELSE '' END) AS TEXT) AS label
FROM (
  SELECT rt.thread_id,
         count(DISTINCT CASE WHEN w.state = 'done' THEN w.uri END) AS done,
         count(DISTINCT CASE WHEN w.state IN ('backlog', 'todo', 'in_progress', 'blocked') THEN w.uri END) AS open
  FROM ref('request_link') rt
  JOIN ref('task') w ON w.uri = rt.task
  GROUP BY rt.thread_id
) n
JOIN ref('thread') th ON th.id = n.thread_id
WHERE n.done > 0 OR n.open > 0
