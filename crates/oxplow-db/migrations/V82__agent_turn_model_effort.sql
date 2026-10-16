-- The model and reasoning effort a turn ran on, as its harness reported
-- them with the turn's end; NULL when it said nothing.
ALTER TABLE agent_turn ADD COLUMN model TEXT;
ALTER TABLE agent_turn ADD COLUMN effort TEXT;
