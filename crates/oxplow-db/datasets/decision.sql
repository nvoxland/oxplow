SELECT d.id, d.thread_id, d.work_item, d.effort_id, d.turn_id, d.question, d.choice,
       d.alternatives_json AS alternatives, d.confidence, d.why, d.provenance, d.created_at,
       CAST(uri('oxplow/decision', 'id', d.id) AS TEXT) AS uri,
       (SELECT e.uri FROM ref('effort') e WHERE e.id = d.effort_id) AS effort_uri,
       (SELECT t.uri FROM ref('thread') t WHERE t.id = d.thread_id) AS thread_uri
FROM source('decision') d
