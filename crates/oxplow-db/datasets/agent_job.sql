SELECT j.id, j.thread_id, j.agent_session_id, j.harness_job_id, j.kind, j.description,
       j.start_call_id,
       CAST(CASE WHEN j.started_turn_id IS NULL THEN NULL
                 ELSE uri('oxplow/turn', 'id', 'trn' || j.started_turn_id) END AS TEXT) AS started_turn,
       j.seen_at,
       CAST(CASE WHEN j.report_turn_id IS NULL THEN NULL
                 ELSE uri('oxplow/turn', 'id', 'trn' || j.report_turn_id) END AS TEXT) AS report_turn,
       j.reported_at, j.status, j.summary,
       CAST(CASE WHEN j.reported_at IS NULL THEN 'running' ELSE 'reported' END AS TEXT) AS state
  FROM source('agent_job') j
