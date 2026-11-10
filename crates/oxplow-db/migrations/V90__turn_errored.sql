-- A turn can end in an error (a harness's API failure, a refused request):
-- outcome `errored` (a value Rust owns, no CHECK), and the harness's own
-- words for it in `error`.
ALTER TABLE agent_turn ADD COLUMN error TEXT;

-- agent.turn.ended@4 -> @5 (an optional `error`): the logged rows take the
-- new version (core keeps only a type's newest version).
UPDATE event_log SET v = 5 WHERE type = 'agent.turn.ended' AND v = 4;
