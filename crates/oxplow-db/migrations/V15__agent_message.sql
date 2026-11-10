-- Messages one agent session sends another, recorded when the sending call
-- is requested, so the receiving turn can be matched to the turn that sent
-- it (the harness gives no message id and no link between them).
--   recipient       who it was sent to, in the harness's words.
--   text_hash       the content hash of its text (event_content's hash).
--   received_turn_id  the turn it was matched to on receipt; NULL until
--                   then, or when it never matched (best effort).
CREATE TABLE agent_message (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    agent_session_id INTEGER REFERENCES agent_session(id) ON DELETE SET NULL,
    turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL,
    recipient TEXT NOT NULL,
    text_hash TEXT NOT NULL,
    sent_at TEXT NOT NULL,
    received_turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL
);
CREATE INDEX idx_agent_message_text ON agent_message(text_hash, sent_at);
