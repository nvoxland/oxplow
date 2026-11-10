-- A request: the turn that started something and every turn that followed
-- from it (a background job's report, a subagent's hand-back), joined by
-- cause links when each turn opens. Identity and membership are stored;
-- what adds up or changes is derived.
--   first_turn_id  the turn that started it; who started it is that
--                  turn's source.
--   agent_turn.request_id  the request a turn belongs to: its cause turn's,
--                  else a new one. NULL for a turn recorded before requests
--                  were (the one-time conversion fills those).
-- Work recorded against a turn (tool calls, tokens, test runs, jobs) is
-- the request's through the turn: no other row repeats it.
CREATE TABLE request (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    first_turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL
);
ALTER TABLE agent_turn ADD COLUMN request_id INTEGER REFERENCES request(id) ON DELETE SET NULL;
CREATE INDEX idx_agent_turn_request ON agent_turn(request_id) WHERE request_id IS NOT NULL;
CREATE INDEX idx_request_thread ON request(thread_id);
