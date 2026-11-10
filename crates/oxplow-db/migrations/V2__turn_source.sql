-- What opened each turn, and the turn whose work it goes on with.
--   source        who opened it: a person's prompt, a background job
--                 reporting back, a subagent's hand-back, another
--                 session's message, or unknown. NULL for a turn recorded
--                 before sources were.
--   harness_turn_id  the harness's own id of the turn (Claude's prompt_id).
--   cause_turn_id the turn whose tool call started the job, or in which
--                 the subagent started.
--   cause_ref     the harness's own name for the cause: the tool call id,
--                 the subagent id, the sender's socket.
--   cause_basis   how the cause is known: reported by the harness, or
--                 matched by oxplow.
--   sender        another session's name for itself, on a peer's turn.
ALTER TABLE agent_turn ADD COLUMN source TEXT
    CHECK (source IN ('person', 'background_job', 'subagent', 'peer', 'unknown'));
ALTER TABLE agent_turn ADD COLUMN harness_turn_id TEXT;
ALTER TABLE agent_turn ADD COLUMN cause_turn_id INTEGER
    REFERENCES agent_turn(id) ON DELETE SET NULL;
ALTER TABLE agent_turn ADD COLUMN cause_ref TEXT;
ALTER TABLE agent_turn ADD COLUMN cause_basis TEXT
    CHECK (cause_basis IN ('reported', 'matched'));
ALTER TABLE agent_turn ADD COLUMN sender TEXT;

-- agent.turn.started@2 → @3: the logged rows gain the source they did not
-- record (core keeps only a type's newest version).
UPDATE event_log
   SET v = 3, payload = json_set(payload, '$.source', 'unknown')
 WHERE type = 'agent.turn.started' AND v = 2;
