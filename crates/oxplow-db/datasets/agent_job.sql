SELECT j.id, j.thread_id, j.agent_session_id, j.harness_job_id, j.kind, j.description,
       j.start_call_id,
       CAST(CASE WHEN j.started_turn_id IS NULL THEN NULL
                 ELSE uri('oxplow/turn', 'id', 'trn' || j.started_turn_id) END AS TEXT) AS started_turn,
       j.start_basis, j.seen_at,
       CAST(CASE WHEN j.report_turn_id IS NULL THEN NULL
                 ELSE uri('oxplow/turn', 'id', 'trn' || j.report_turn_id) END AS TEXT) AS report_turn,
       j.reported_at, j.status, j.summary, j.lost_at,
       CAST(CASE WHEN j.reported_at IS NOT NULL THEN 'reported'
                 WHEN j.lost_at IS NOT NULL THEN 'lost'
                 ELSE 'running' END AS TEXT) AS state
  FROM source('agent_job') j
