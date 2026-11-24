-- How a final message is worded no longer says the thread waits on the
-- person: only a question or plan put to them, or a permission prompt,
-- does. The question a turn's final message ended on is no longer kept.
ALTER TABLE agent_turn DROP COLUMN question;
