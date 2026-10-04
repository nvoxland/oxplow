-- Each effort's latest verdict, as its chip and rows show it.
SELECT v.ref,
       CAST(CASE WHEN v.verdict = 'changes_requested' THEN 'Changes requested'
                 WHEN v.forced = 1 THEN 'Accepted (forced)'
                 ELSE 'Accepted' END AS TEXT) AS label,
       CAST(CASE WHEN v.verdict = 'changes_requested' THEN 'red'
                 WHEN v.forced = 1 THEN 'orange'
                 ELSE 'green' END AS TEXT) AS color
FROM ref('verdicts') v
WHERE v.seq = (SELECT max(l.seq) FROM ref('verdicts') l WHERE l.ref = v.ref)
