-- Every verdict a reviewer gave, appended as each lands. It reads only
-- the envelope — the type says the verdict, the subject what it was about
-- (the effort first) — which retention keeps, so a verdict outlives its
-- payload (tsk886). An acceptance whose subject names a claim or a
-- decision accepted it unchecked: forced.
SELECT e.seq,
       CAST(json_extract(e.subject, '$[0]') AS TEXT) AS ref,
       CAST(CASE e.type WHEN 'oxplow_review.accepted' THEN 'accepted'
                        ELSE 'changes_requested' END AS TEXT) AS verdict,
       instr(e.subject, '"claim:') > 0 OR instr(e.subject, '"decision:') > 0 AS forced
FROM ref('event') e
WHERE e.type IN ('oxplow_review.accepted', 'oxplow_review.changes_requested')
