-- Files each effort changed outside the area its work item names. A file
-- is in the area when the item's title or body names it, or names one of
-- its directories at least two levels deep (`src/ui`). An item that names
-- no area has no deviations.
WITH RECURSIVE
files AS (
  SELECT effort_id, path, change_kind FROM ref('effort_file')
),
area AS (
  SELECT e.id AS effort_id, w.title || ' ' || coalesce(w.body, '') AS text
  FROM ref('effort') e JOIN ref('work_item') w ON w.ref = e.work_item
),
-- Every directory prefix of every file (`a/`, `a/b/`, …).
dirs(effort_id, path, prefix, rest) AS (
  SELECT effort_id, path, '', path FROM files
  UNION ALL
  SELECT effort_id, path, prefix || substr(rest, 1, instr(rest, '/')), substr(rest, instr(rest, '/') + 1)
  FROM dirs WHERE instr(rest, '/') > 0
),
scored AS (
  SELECT f.effort_id, f.path, f.change_kind,
         instr(a.text, f.path) > 0
         OR EXISTS (
           SELECT 1 FROM dirs d
           WHERE d.effort_id = f.effort_id AND d.path = f.path
             AND length(d.prefix) - length(replace(d.prefix, '/', '')) >= 2
             AND instr(a.text, rtrim(d.prefix, '/')) > 0
         ) AS in_area
  FROM files f JOIN area a ON a.effort_id = f.effort_id
)
SELECT s.effort_id, s.path, s.change_kind
FROM scored s
WHERE s.in_area = 0
  AND EXISTS (SELECT 1 FROM scored o WHERE o.effort_id = s.effort_id AND o.in_area = 1)
