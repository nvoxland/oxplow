-- Each effort that changed a migration file, judged by this policy:
-- its own facts (the DDL in those files), core's test evidence (the
-- latest run, the diff coverage) and core's code analysis (the
-- functions it added).
WITH schema AS (
  SELECT ef.effort_id,
         sum(t.value) AS statements,
         sum(CASE WHEN json_extract(t.dims_json, '$."schema_review.ddl_kind"') = 'drop'
                  THEN t.value ELSE 0 END) AS drops
  FROM ref('effort_file') ef
  JOIN ref('tree_fact') t ON t.path = ef.path AND t.measure_key = 'schema_review.ddl'
  GROUP BY ef.effort_id
),
last_run AS (
  SELECT r.effort_id, r.failed, r.total, r.provenance
  FROM ref('test_run') r
  WHERE r.id = (SELECT l.id FROM ref('test_run') l WHERE l.effort_id = r.effort_id
                ORDER BY l.captured_at DESC, l.id DESC LIMIT 1)
),
coverage AS (
  SELECT o.effort_id, o.metric_value AS pct
  FROM ref('effort_observation') o
  WHERE o.kind = 'diff-coverage'
    AND o.seq = (SELECT max(l.seq) FROM ref('effort_observation') l
                 WHERE l.effort_id = o.effort_id AND l.kind = 'diff-coverage')
),
complex AS (
  SELECT CAST(c.target AS INTEGER) AS effort_id, max(f.complexity) AS worst
  FROM ref('change') c JOIN ref('change_function') f ON f.change_id = c.id
  WHERE c.kind = 'effort' AND f.status = 'added'
  GROUP BY c.target
),
judged AS (
  SELECT s.effort_id, e.ref, s.statements,
         rtrim(
           CASE WHEN s.drops > 0 THEN 'drops a table; ' ELSE '' END
           || CASE WHEN r.effort_id IS NULL THEN 'no test run; '
                   WHEN r.provenance <> 'observed' THEN 'its last test run was only reported; '
                   WHEN coalesce(r.failed, 0) > 0 OR coalesce(r.total, 0) = 0 THEN 'its last test run wasn''t green; '
                   ELSE '' END
           || CASE WHEN cv.pct < 90 THEN 'diff coverage ' || CAST(round(cv.pct) AS INTEGER) || '% (under 90%); ' ELSE '' END
           || CASE WHEN cx.worst > 15 THEN 'a new function of complexity ' || CAST(cx.worst AS INTEGER) || '; ' ELSE '' END,
           '; ') AS reasons
  FROM schema s
  JOIN ref('effort') e ON e.id = s.effort_id
  LEFT JOIN last_run r ON r.effort_id = s.effort_id
  LEFT JOIN coverage cv ON cv.effort_id = s.effort_id
  LEFT JOIN complex cx ON cx.effort_id = s.effort_id
)
SELECT effort_id, ref,
       CAST(CASE WHEN reasons = '' THEN 'accepted' ELSE 'changes requested' END AS TEXT) AS verdict,
       reasons, statements,
       CAST(CASE WHEN reasons = '' THEN 'Schema OK' ELSE 'Schema: changes' END AS TEXT) AS label,
       CAST(CASE WHEN reasons = '' THEN 'green' ELSE 'red' END AS TEXT) AS color
FROM judged
