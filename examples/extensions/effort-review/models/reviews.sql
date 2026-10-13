-- Each effort review: the job, the effort its intent key names, the
-- effort's work item, and the text its current attempt has streamed.
SELECT j.id AS job,
       CAST(substr(j.intent_key, length('review:') + 1) AS TEXT) AS effort,
       e.work_item,
       j.state,
       j.updated_at AS reviewed_at,
       CAST(coalesce((SELECT group_concat(o.text, '')
                        FROM (SELECT text FROM ref('job_output')
                               WHERE job_id = j.id AND attempt = j.attempt AND kind = 'text'
                               ORDER BY seq) o), '') AS TEXT) AS review
FROM ref('job') j
LEFT JOIN ref('effort') e ON 'effort:eff' || e.id = substr(j.intent_key, length('review:') + 1)
WHERE j.job = 'effort-review/job/review'
