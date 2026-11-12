-- Files each request changed outside the area its tasks name. A file
-- is in the area when a task's title or body names it, or names one of
-- its directories at least two levels deep (`src/ui`). A request whose
-- tasks name no area has no deviations. A request's files are its turns'
-- files that a call of the turn claimed or that no other turn ran beside.
WITH RECURSIVE
files AS (
  SELECT DISTINCT t.request_id, f.path
  FROM ref('turn_file') f JOIN ref('agent_turn') t ON t.id = f.turn_id
  WHERE t.request_id IS NOT NULL AND f.confidence IN ('claimed', 'sole')
),
area AS (
  SELECT q.request_id, group_concat(w.title || ' ' || coalesce(w.body, ''), ' ') AS text
  FROM (SELECT DISTINCT request_id, task FROM ref('request_task')) q
  JOIN ref('task') w ON w.uri = q.task
  GROUP BY q.request_id
),
-- Every directory prefix of every file (`a/`, `a/b/`, …).
dirs(request_id, path, prefix, rest) AS (
  SELECT request_id, path, '', path FROM files
  UNION ALL
  SELECT request_id, path, prefix || substr(rest, 1, instr(rest, '/')), substr(rest, instr(rest, '/') + 1)
  FROM dirs WHERE instr(rest, '/') > 0
),
scored AS (
  SELECT f.request_id, f.path,
         instr(a.text, f.path) > 0
         OR EXISTS (
           SELECT 1 FROM dirs d
           WHERE d.request_id = f.request_id AND d.path = f.path
             AND length(d.prefix) - length(replace(d.prefix, '/', '')) >= 2
             AND instr(a.text, rtrim(d.prefix, '/')) > 0
         ) AS in_area
  FROM files f JOIN area a ON a.request_id = f.request_id
)
SELECT s.request_id, s.path
FROM scored s
WHERE s.in_area = 0
  AND EXISTS (SELECT 1 FROM scored o WHERE o.request_id = s.request_id AND o.in_area = 1)
