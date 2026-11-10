-- How each turn ended, and what it left running.
--   outcome       completed, interrupted (the person), session_ended (its
--                 harness session ended mid-turn), session_restarted (a new
--                 harness process started with it open), session_closed
--                 (oxplow closed the agent session), restart (oxplow's own
--                 recovery). NULL while running.
--   left_running  JSON array of the jobs and subagents still running when
--                 it ended ({id, kind?, description?}), as its harness
--                 reported; NULL when the harness said nothing.
ALTER TABLE agent_turn ADD COLUMN outcome TEXT
    CHECK (outcome IN ('completed', 'interrupted', 'session_ended', 'session_restarted',
                       'session_closed', 'restart'));
ALTER TABLE agent_turn ADD COLUMN left_running TEXT;

-- Ended turns take the outcome their agent.turn.ended logged; the three
-- ends logged as `interrupted` are told apart by the text oxplow wrote
-- into the answer, which then goes: `answer` is only what the agent said.
UPDATE agent_turn
   SET outcome = (SELECT json_extract(e.payload, '$.outcome') FROM event_log e
                   WHERE e.type = 'agent.turn.ended' AND e.turn_id = agent_turn.id
                   ORDER BY e.seq DESC LIMIT 1)
 WHERE ended_at IS NOT NULL;
UPDATE agent_turn
   SET outcome = CASE answer
                   WHEN 'session ended' THEN 'session_ended'
                   WHEN 'session restarted' THEN 'session_restarted'
                   WHEN 'session closed' THEN 'session_closed'
                   ELSE outcome END
 WHERE outcome = 'interrupted';
UPDATE agent_turn SET answer = NULL
 WHERE (outcome = 'interrupted' AND answer = 'interrupted')
    OR (outcome = 'session_ended' AND answer = 'session ended')
    OR (outcome = 'session_restarted' AND answer = 'session restarted')
    OR (outcome = 'session_closed' AND answer = 'session closed')
    OR (outcome = 'restart' AND answer = 'interrupted_by_restart');

-- agent.turn.ended@3 → @4: the logged rows take their turn's outcome
-- (core keeps only a type's newest version).
UPDATE event_log
   SET v = 4,
       payload = json_set(payload, '$.outcome',
                          coalesce((SELECT t.outcome FROM agent_turn t WHERE t.id = event_log.turn_id),
                                   json_extract(payload, '$.outcome')))
 WHERE type = 'agent.turn.ended' AND v = 3;
