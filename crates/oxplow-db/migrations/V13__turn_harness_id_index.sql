-- A child's `rootTurnId` names its spawning turn by the harness's own id:
-- `turn_by_harness_id_tx` looks a turn up by it.
CREATE INDEX idx_agent_turn_harness_id ON agent_turn(thread_id, harness_turn_id)
    WHERE harness_turn_id IS NOT NULL;
