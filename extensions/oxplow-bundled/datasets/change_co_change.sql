-- A change's surprising files. Dormant first: no commit touched it in
-- `coChange.dormantDays` days (never touched counts as that many). Otherwise, its usual partners — its
-- top three co-changers in `co_change_pair` — are all absent from the
-- change.
-- The last commit touching each changed path is found once (a per-row
-- subquery scanned every commit file for each changed file), and the pairs are
-- built once per statement (`MATERIALIZED`).
WITH touched AS MATERIALIZED (
  SELECT f.path, max(julianday(c.committed_at)) AS last_touched
    FROM ref('commit_file') f JOIN ref('commit') c ON c.sha = f.sha
   WHERE f.path IN (SELECT path FROM ref('change_file'))
   GROUP BY f.path
),
files AS (
  SELECT cf.change_id, cf.path, t.last_touched
  FROM ref('change_file') cf LEFT JOIN touched t ON t.path = cf.path
),
idle AS (
  SELECT change_id, path,
         CASE WHEN last_touched IS NULL THEN config('/coChange/dormantDays')
              ELSE CAST(julianday('now') - last_touched AS INTEGER) END AS days
  FROM files
),
pairs AS MATERIALIZED (SELECT path, other, together FROM ref('co_change_pair')),
usual AS (
  SELECT i.change_id, i.path, p.other, p.together,
         row_number() OVER (PARTITION BY i.change_id, i.path
                            ORDER BY p.together DESC, p.other) AS rank
  FROM idle i JOIN pairs p ON p.path = i.path
  WHERE i.days < config('/coChange/dormantDays')
)
SELECT change_id, path, 'dormant' AS reason, NULL AS expected, days AS dormant_days
FROM idle WHERE days >= config('/coChange/dormantDays')
UNION ALL
SELECT u.change_id, u.path, 'usual-co-changers-absent',
       group_concat(u.other, ', ' ORDER BY u.together DESC, u.other), NULL
FROM usual u
WHERE u.rank <= 3
GROUP BY u.change_id, u.path
HAVING sum(EXISTS (SELECT 1 FROM ref('change_file') o
                   WHERE o.change_id = u.change_id AND o.path = u.other)) = 0
