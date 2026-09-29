SELECT id, thread_id, task_id, effort_id, turn_id, question, choice,
       alternatives_json AS alternatives, confidence, why, provenance, created_at
FROM source('decision')
