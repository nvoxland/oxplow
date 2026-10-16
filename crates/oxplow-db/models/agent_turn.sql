SELECT t.id, t.thread_id, t.agent_session_id, t.prompt, t.answer, t.session_id, t.started_at,
       t.ended_at, t.start_snapshot_id, t.snapshot_id,
       CAST('turn:trn' || t.id AS TEXT) AS ref,
       -- The session's transcript, opened at this turn: where a turn opens.
       -- NULL for a turn no session claims.
       CAST(CASE WHEN t.agent_session_id IS NULL THEN NULL ELSE lens_ref(
         'oxplow-foundation/session-transcript',
         'session', (SELECT s.ref FROM ref('agent_session') s WHERE s.id = t.agent_session_id),
         'turn', 'turn:trn' || t.id) END AS TEXT) AS transcript_page,
       t.model, t.effort, t.context_tokens, t.context_window
  FROM source('agent_turn') t
