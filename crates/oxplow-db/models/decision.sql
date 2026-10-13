SELECT id, thread_id, work_item, effort_id, turn_id, question, choice,
       alternatives_json AS alternatives, confidence, why, provenance, created_at,
       CAST('decision:' || id AS TEXT) AS ref
FROM source('decision')
