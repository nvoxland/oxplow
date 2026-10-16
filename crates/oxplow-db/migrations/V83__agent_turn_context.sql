-- How full the agent's context was when a turn ended, and the size of its
-- window, as its harness reported them with the turn's end; NULL when it
-- said nothing.
ALTER TABLE agent_turn ADD COLUMN context_tokens INTEGER;
ALTER TABLE agent_turn ADD COLUMN context_window INTEGER;
