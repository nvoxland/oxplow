-- What a person's run acted through, when not directly: the Assistant
-- session that ran the command for them (`assistant:ast3`). NULL for a
-- run the actor made itself.
ALTER TABLE command_audit ADD COLUMN via TEXT;
