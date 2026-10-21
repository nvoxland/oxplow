-- Each effort review: the run of the review job, the effort its intent key
-- (`review:<effort ref>`) names, the effort's work item, and the text its
-- current attempt has streamed.
SELECT j.ref AS job_run,
       CAST(substr(j.intent_key, length('review:') + 1) AS TEXT) AS effort,
       e.work_item,
       j.state,
       j.updated_at AS reviewed_at,
       CAST(coalesce((SELECT group_concat(o.text, '')
                        FROM (SELECT text FROM ref('job_run_output')
                               WHERE job_id = j.id AND attempt = j.attempt AND kind = 'text'
                               ORDER BY seq) o), '') AS TEXT) AS review
FROM ref('job_run') j
LEFT JOIN ref('effort') e ON e.ref = substr(j.intent_key, length('review:') + 1)
WHERE j.job = uri('oxplow/job', 'job', 'effort-review/review')
