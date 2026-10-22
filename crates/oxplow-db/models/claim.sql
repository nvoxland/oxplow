SELECT c.id, c.thread_id, c.work_item, c.effort_id, c.turn_id, c.agent_session_id, c.statement, c.kind,
       c.evidence, c.review, c.provenance,
       c.created_at,
       CAST(uri('oxplow/claim', 'id', c.id) AS TEXT) AS uri
FROM source('claim') c
