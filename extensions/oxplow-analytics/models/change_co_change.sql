-- A change's surprising files. Dormant first: no commit touched it in 90
-- days (never touched counts as 90). Otherwise, its usual partners — its
-- top three co-changers in `co_change_pair` — are all absent from the
-- change.
WITH files AS (
  SELECT cf.change_id, cf.path,
         (SELECT max(julianday(c.committed_at))
            FROM ref('commit_file') f JOIN ref('commit') c ON c.sha = f.sha
           WHERE f.path = cf.path) AS last_touched
  FROM ref('change_file') cf
),
idle AS (
  SELECT change_id, path,
         CASE WHEN last_touched IS NULL THEN 90
              ELSE CAST(julianday('now') - last_touched AS INTEGER) END AS days
  FROM files
),
usual AS (
  SELECT i.change_id, i.path, p.other, p.together,
         row_number() OVER (PARTITION BY i.change_id, i.path
                            ORDER BY p.together DESC, p.other) AS rank
  FROM idle i JOIN ref('co_change_pair') p ON p.path = i.path
  WHERE i.days < 90
)
SELECT change_id, path, 'dormant' AS reason, NULL AS expected, days AS dormant_days
FROM idle WHERE days >= 90
UNION ALL
SELECT u.change_id, u.path, 'usual-co-changers-absent',
       group_concat(u.other, ', ' ORDER BY u.together DESC, u.other), NULL
FROM usual u
WHERE u.rank <= 3
GROUP BY u.change_id, u.path
HAVING sum(EXISTS (SELECT 1 FROM ref('change_file') o
                   WHERE o.change_id = u.change_id AND o.path = u.other)) = 0
