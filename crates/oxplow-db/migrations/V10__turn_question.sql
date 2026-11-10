-- The question a turn's final message ended on (its last line, when the
-- message ends in a question mark), set when the turn closes; NULL when it
-- ended on none, or for a turn closed before this was recorded (the
-- one-time conversion fills those).
ALTER TABLE agent_turn ADD COLUMN question TEXT;
