SELECT t.id, t.thread_id, t.agent_session_id, t.prompt, t.answer, t.session_id, t.started_at,
       t.ended_at,
       CAST(uri('oxplow/turn', 'id', 'trn' || t.id) AS TEXT) AS uri,
       -- The session's transcript (its `session`), shown from this turn (its
       -- view's `turn`): where a turn opens. NULL for a turn no session claims.
       CAST(CASE WHEN t.agent_session_id IS NULL THEN NULL ELSE uri_view(uri('oxplow/page',
         'page', 'session-transcript',
         'session', (SELECT uri_param(s.uri, 'id') FROM ref('agent_session') s
                      WHERE s.id = t.agent_session_id)),
         'turn', 'trn' || t.id) END AS TEXT) AS transcript_page,
       t.model, t.effort, t.context_tokens, t.context_window,
       t.source, t.harness_turn_id,
       CAST(CASE WHEN t.cause_turn_id IS NULL THEN NULL
                 ELSE uri('oxplow/turn', 'id', 'trn' || t.cause_turn_id) END AS TEXT) AS cause_turn,
       t.cause_ref, t.cause_basis, t.sender, t.outcome, t.error,
       CAST(CASE WHEN t.request_id IS NULL THEN NULL
                 ELSE uri('oxplow/request', 'id', 'req' || t.request_id) END AS TEXT) AS request,
       CAST(t.request_id AS INT) AS request_id
  FROM source('agent_turn') t
