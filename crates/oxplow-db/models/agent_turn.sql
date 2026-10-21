SELECT t.id, t.thread_id, t.agent_session_id, t.prompt, t.answer, t.session_id, t.started_at,
       t.ended_at, t.start_snapshot_id, t.snapshot_id,
       CAST(uri('oxplow/turn', 'id', 'trn' || t.id) AS TEXT) AS ref,
       -- The session's transcript, opened at this turn: where a turn opens.
       -- NULL for a turn no session claims.
       CAST(CASE WHEN t.agent_session_id IS NULL THEN NULL ELSE uri('oxplow/lens',
         'lens', 'oxplow-foundation/session-transcript',
         'session', (SELECT s.ref FROM ref('agent_session') s WHERE s.id = t.agent_session_id),
         'turn', uri('oxplow/turn', 'id', 'trn' || t.id)) END AS TEXT) AS transcript_page,
       t.model, t.effort, t.context_tokens, t.context_window,
       -- Who opened a turn with no prompt of a person's: the other agent
       -- session whose message it was. NULL for every other turn.
       CAST((SELECT json_extract(e.payload, '$.from') FROM source('event_log') e
              WHERE t.prompt = '' AND e.type = 'agent.peer_message.received' AND e.turn_id = t.id
              ORDER BY e.seq LIMIT 1) AS TEXT) AS from_peer
  FROM source('agent_turn') t
