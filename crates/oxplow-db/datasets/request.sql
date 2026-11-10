-- Each request: who started it and when, from its first turn.
SELECT r.id, r.thread_id,
       CAST(uri('oxplow/request', 'id', 'req' || r.id) AS TEXT) AS uri,
       CAST(CASE WHEN r.first_turn_id IS NULL THEN NULL
                 ELSE uri('oxplow/turn', 'id', 'trn' || r.first_turn_id) END AS TEXT) AS first_turn,
       t.source, t.prompt, t.started_at
  FROM source('request') r
  LEFT JOIN source('agent_turn') t ON t.id = r.first_turn_id
