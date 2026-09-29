-- Review fix (tsk438) — a turn's diff is the snapshot it STARTED at →
-- the snapshot it ENDED at.
--
-- V98 described "what a turn changed" as its turn_end op's parent →
-- snapshot, but that parent is whatever the stream's current snapshot
-- was when the turn ended: any take during the turn (an effort_end from
-- complete_task, a git_refs take on a commit, another thread's turn end)
-- moves it, so the usual flow — edit, complete_task, Stop — showed a turn
-- that changed nothing. `start_snapshot_id` is recorded in the turn's open
-- transaction (the stream's current snapshot at that moment), exactly as
-- efforts are bracketed.

ALTER TABLE agent_turn ADD COLUMN start_snapshot_id INTEGER REFERENCES snapshot(id) ON DELETE SET NULL;

DROP VIEW v_agent_turn;
CREATE VIEW v_agent_turn AS
SELECT id, thread_id, prompt, answer, session_id, started_at, ended_at,
       start_snapshot_id, snapshot_id
  FROM agent_turn;
