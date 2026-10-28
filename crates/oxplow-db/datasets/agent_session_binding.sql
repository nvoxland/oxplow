SELECT b.id, b.agent_session_id, b.native_id, b.origin, b.bound_at, b.unbound_at,
       (SELECT s.uri FROM ref('agent_session') s WHERE s.id = b.agent_session_id) AS session_uri
FROM source('agent_session_binding') b
