SELECT c.id, c.thread_id, c.work_item, c.effort_id, c.turn_id, c.agent_session_id, c.statement, c.kind,
       c.evidence_ref, c.review, c.provenance,
       c.created_at,
       CAST('claim:' || c.id AS TEXT) AS ref
FROM source('claim') c
