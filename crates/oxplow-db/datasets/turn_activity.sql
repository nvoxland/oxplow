-- What happened inside each turn, counted from its events (anchored to the
-- turn): kept apart from v_agent_turn, which many pages read whole.
SELECT t.id AS turn_id, t.thread_id,
       CAST(uri('oxplow/turn', 'id', 'trn' || t.id) AS TEXT) AS turn,
       CAST((SELECT count(*) FROM source('event_log') e
              WHERE e.turn_id = t.id AND e.type = 'agent.context.compacted') AS INTEGER) AS compactions,
       CAST((SELECT count(*) FROM source('event_log') e
              WHERE e.turn_id = t.id AND e.type = 'agent.prompt.submitted') AS INTEGER) AS deliveries,
       CAST((SELECT count(*) FROM source('event_log') e
              WHERE e.turn_id = t.id AND e.type = 'agent.prompt.submitted'
                AND json_extract(e.payload, '$.source') = 'person') AS INTEGER) AS person_prompts
  FROM source('agent_turn') t
