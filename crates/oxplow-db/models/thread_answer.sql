SELECT id, CAST(uri('oxplow/answer', 'id', id) AS TEXT) AS uri, thread_id, turn_id, effort_id, title,
       lens, spec, params, created_at, kept_lens
FROM source('thread_answer')
