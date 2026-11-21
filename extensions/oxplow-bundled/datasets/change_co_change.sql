-- A change's surprising files. Dormant first: no commit touched it in
-- `coChange.dormantDays` days (never touched counts as that many). Otherwise, its usual partners — its
-- top three co-changers in `co_change_pair` — are all absent from the
-- change.
-- One select over the change's own files: the last commit touching a path
-- and its top partners are looked up by path (the commit index and the pair
-- table are indexed by it), so a filter on `change_id` reads only that
-- change's files. A CTE read twice (a dormant arm and a partners arm) would
-- be computed for every change first.
SELECT i.change_id, i.path,
       CASE WHEN i.days >= config('/coChange/dormantDays') THEN 'dormant'
            ELSE 'usual-co-changers-absent' END AS reason,
       CASE WHEN i.days >= config('/coChange/dormantDays') THEN NULL
            ELSE (SELECT group_concat(t.other, ', ' ORDER BY t.together DESC, t.other)
                    FROM (SELECT p.other, p.together FROM ref('co_change_pair') p
                           WHERE p.path = i.path
                           ORDER BY p.together DESC, p.other LIMIT 3) t) END AS expected,
       CASE WHEN i.days >= config('/coChange/dormantDays') THEN i.days END AS dormant_days
FROM (
  SELECT d.change_id, d.path,
         CASE WHEN d.last_touched IS NULL THEN config('/coChange/dormantDays')
              ELSE CAST(julianday('now') - d.last_touched AS INTEGER) END AS days
  FROM (
    SELECT cf.change_id, cf.path,
           (SELECT max(julianday(c.committed_at))
              FROM ref('commit_file') f JOIN ref('commit') c ON c.sha = f.sha
             WHERE f.path = cf.path) AS last_touched
    FROM ref('change_file') cf
  ) d
) i
WHERE i.days >= config('/coChange/dormantDays')
   OR (EXISTS (SELECT 1 FROM ref('co_change_pair') p WHERE p.path = i.path)
       AND NOT EXISTS (
         SELECT 1
           FROM (SELECT p.other FROM ref('co_change_pair') p
                  WHERE p.path = i.path
                  ORDER BY p.together DESC, p.other LIMIT 3) t
           JOIN ref('change_file') o ON o.change_id = i.change_id AND o.path = t.other))
