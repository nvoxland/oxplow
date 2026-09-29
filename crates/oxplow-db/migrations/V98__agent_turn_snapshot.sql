-- P2.3 (tsk425) — turns on the timeline (.context/target-architecture.md
-- §4.3; .context/data-model.md "agent_turn").
--
-- A turn now ends with a snapshot: the `turn_end` take the Stop hook runs
-- records the snapshot the worktree was at when the turn finished, and
-- `agent_turn.snapshot_id` points at it (set in the take's transaction;
-- the take's own `snapshot_op` row carries its parent, so "what changed
-- this turn" is parent → snapshot).
--
-- `agent_turn.task_id` goes: nothing ever set it (hook ingest always wrote
-- NULL), and a turn belongs to efforts by time, not to one task.
--
-- `event_log` gets a `turn_id` index now that `agent.turn.*` and turn-end
-- `snapshot.taken` events carry the anchor.

DROP VIEW v_agent_turn;
DROP INDEX idx_agent_turn_task;
ALTER TABLE agent_turn DROP COLUMN task_id;
ALTER TABLE agent_turn ADD COLUMN snapshot_id INTEGER REFERENCES snapshot(id) ON DELETE SET NULL;
CREATE INDEX idx_agent_turn_snapshot ON agent_turn (snapshot_id) WHERE snapshot_id IS NOT NULL;

CREATE VIEW v_agent_turn AS
SELECT id, thread_id, prompt, answer, session_id, started_at, ended_at, snapshot_id
  FROM agent_turn;

CREATE INDEX event_log_turn ON event_log (turn_id, seq) WHERE turn_id IS NOT NULL;
