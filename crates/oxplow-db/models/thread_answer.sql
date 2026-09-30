SELECT id, CAST('answer:' || id AS TEXT) AS ref, thread_id, turn_id, effort_id, title,
       lens, spec, params, created_at, kept_lens
FROM source('thread_answer')
