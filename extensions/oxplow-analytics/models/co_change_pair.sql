-- Files committed together: each pair, both ways round, that shared at
-- least 3 commits in the last 180 days — counting only commits of 50
-- files or fewer, since a mass rename or formatter sweep would drown the
-- signal. Materialized: refilled when the commit index moves.
WITH recent AS (
  SELECT f.sha, f.path
  FROM ref('commit_file') f JOIN ref('commit') c ON c.sha = f.sha
  WHERE julianday(c.committed_at) >= julianday('now', '-180 days')
),
sized AS (SELECT sha FROM recent GROUP BY sha HAVING count(*) <= 50)
SELECT a.path AS path, b.path AS other, count(*) AS together
FROM recent a JOIN recent b ON b.sha = a.sha AND b.path <> a.path
WHERE a.sha IN (SELECT sha FROM sized)
GROUP BY a.path, b.path
HAVING count(*) >= 3
