SELECT id, thread_id, agent_session_id, prompt, answer, session_id, started_at, ended_at,
       start_snapshot_id, snapshot_id,
       CAST('turn:trn' || id AS TEXT) AS ref
  FROM source('agent_turn')
