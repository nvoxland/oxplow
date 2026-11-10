-- A turn can end in an error (a harness's API failure, a refused request):
-- outcome `errored`, and the harness's own words for it in `error`.
-- The outcome column's CHECK lists the outcomes, and SQLite cannot change a
-- CHECK in place: the column is dropped and added again, its values kept.
CREATE TEMP TABLE turn_outcome_kept AS
    SELECT id, outcome FROM agent_turn WHERE outcome IS NOT NULL;
ALTER TABLE agent_turn DROP COLUMN outcome;
ALTER TABLE agent_turn ADD COLUMN outcome TEXT
    CHECK (outcome IN ('completed', 'interrupted', 'errored', 'session_ended',
                       'session_restarted', 'session_closed', 'restart'));
UPDATE agent_turn
   SET outcome = (SELECT k.outcome FROM turn_outcome_kept k WHERE k.id = agent_turn.id)
 WHERE id IN (SELECT id FROM turn_outcome_kept);
DROP TABLE turn_outcome_kept;
ALTER TABLE agent_turn ADD COLUMN error TEXT;

-- agent.turn.ended@4 → @5 (an optional `error`): the logged rows take the
-- new version (core keeps only a type's newest version).
UPDATE event_log SET v = 5 WHERE type = 'agent.turn.ended' AND v = 4;
