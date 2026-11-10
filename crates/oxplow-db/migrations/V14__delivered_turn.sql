-- The turn a delivery to the agent landed in: a handoff, or an item of the
-- context basket, given with the prompt that opened (or went into) the turn.
-- Delivered ones are also `prompt_mention` links from that turn.
ALTER TABLE handoff ADD COLUMN delivered_turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL;
ALTER TABLE thread_context_item ADD COLUMN delivered_turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL;
