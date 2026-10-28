-- Every verdict a reviewer gave, appended as each lands. It reads only
-- the envelope — the type says the verdict, the subject what it was about
-- (the effort first) — which retention keeps, so a verdict outlives its
-- payload (tsk886). An acceptance whose subject names a claim or a
-- decision accepted it unchecked: forced. (An incremental dataset can't scan
-- the subject with a subquery: it matches the resources' URI prefix.)
SELECT e.seq,
       CAST(json_extract(e.subject, '$[0]') AS TEXT) AS uri,
       CAST(CASE e.type WHEN 'oxplow_bundled.accepted' THEN 'accepted'
                        ELSE 'changes_requested' END AS TEXT) AS verdict,
       instr(e.subject, '"oxplow:oxplow/claim?') > 0
         OR instr(e.subject, '"oxplow:oxplow/decision?') > 0 AS forced
FROM ref('event') e
WHERE e.type IN ('oxplow_bundled.accepted', 'oxplow_bundled.changes_requested')
