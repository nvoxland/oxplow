SELECT b.id, b.agent_session_id, b.native_id, b.origin, b.bound_at, b.unbound_at,
       (SELECT s.ref FROM ref('agent_session') s WHERE s.id = b.agent_session_id) AS session_ref
FROM source('agent_session_binding') b
