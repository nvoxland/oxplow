-- P10.M1 (tsk483): a capture carries the agent turn it was measured in —
-- the turn its causing tool event was anchored to, or the thread's open
-- turn for a run an agent reported by command. NULL for a capture no turn
-- produced (a scheduled collector, a baseline scan). The capture outlives
-- its turn (SET NULL).
ALTER TABLE metric_capture
    ADD COLUMN turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL;
CREATE INDEX idx_metric_capture_turn ON metric_capture (turn_id) WHERE turn_id IS NOT NULL;
