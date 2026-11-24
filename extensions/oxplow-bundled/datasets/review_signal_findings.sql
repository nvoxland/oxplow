-- Lint-like findings on the lines a change touched: facts with a rule or a
-- severity at a line (the language packs' idiom collectors, a project's own
-- fact collectors) and analyzer findings (a report collector's
-- static-analysis observation, for a request). Facts are the change's own:
-- the tree's facts as of its head snapshot (`v_change_fact`), so a change
-- headed at an older snapshot still has the findings it had then. A change
-- headed at a commit has no snapshot to read them at and gets analyzer
-- findings only.
-- A fact is at its `line`, or, for a fact per file (the packs' idiom
-- collectors: one count per file, so the metric cube stays small), at each
-- line its `detail` lists (`{"lines": [3, 17]}`).
WITH located AS (
  SELECT cf.change_id, cf.path, coalesce(cf.line, l.value) AS line, cf.rule, cf.severity,
         CASE WHEN cf.line IS NULL AND json_valid(cf.detail) THEN NULL ELSE cf.detail END AS detail,
         cf.measure_key, cf.subject
  FROM ref('change_fact') cf
  LEFT JOIN json_each(CASE WHEN cf.line IS NULL AND json_valid(cf.detail) THEN cf.detail END, '$.lines') l
  WHERE cf.rule IS NOT NULL OR cf.severity IS NOT NULL
),
facts AS (
  SELECT located.*
  FROM located
  JOIN ref('change_hunk') hk ON hk.change_id = located.change_id AND hk.path = located.path
   AND located.line BETWEEN hk.head_start AND hk.head_end
  WHERE located.line IS NOT NULL
),
analysis AS (
  SELECT c.id AS change_id, o.payload_json,
         row_number() OVER (PARTITION BY c.id ORDER BY o.seq DESC) AS nth
  FROM ref('change') c
  JOIN ref('request_observation') o
    ON c.kind = 'request' AND o.request_id = CAST(c.target AS INTEGER) AND o.kind = 'static-analysis'
  WHERE c.status = 'done'
),
findings AS (
  SELECT a.change_id, json_extract(x.value, '$.path') AS path,
         json_extract(x.value, '$.line') AS line,
         json_extract(x.value, '$.rule') AS rule,
         json_extract(x.value, '$.severity') AS severity,
         json_extract(x.value, '$.message') AS message
  FROM analysis a, json_each(a.payload_json, '$.findings') AS x
  WHERE a.nth = 1
)
SELECT DISTINCT facts.change_id, 'oxplow/review_signal_findings' AS source, 'code' AS area,
       'lint-' || coalesce(facts.rule, facts.measure_key) AS signal,
       CASE facts.severity WHEN 'error' THEN 'high' WHEN 'info' THEN 'info' WHEN 'note' THEN 'info'
                           ELSE 'medium' END AS severity,
       facts.path, facts.line, NULL AS end_line,
       'Added: ' || coalesce(facts.rule, facts.measure_key) AS title,
       coalesce(facts.detail, printf('A changed line has `%s`.', coalesce(facts.rule, facts.measure_key))) AS why,
       NULL AS uri
FROM facts
UNION ALL
SELECT findings.change_id, 'oxplow/review_signal_findings', 'code',
       'analyzer-' || coalesce(findings.rule, findings.severity),
       CASE findings.severity WHEN 'error' THEN 'high' WHEN 'warning' THEN 'medium' ELSE 'info' END,
       findings.path, findings.line, NULL,
       coalesce(findings.rule, 'Analyzer finding') || ' on a changed line',
       findings.message, NULL
FROM findings
JOIN ref('change_hunk') hk ON hk.change_id = findings.change_id AND hk.path = findings.path
 AND findings.line BETWEEN hk.head_start AND hk.head_end
