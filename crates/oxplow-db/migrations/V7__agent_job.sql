-- A job an agent started that runs on after the call that started it (a
-- background shell, a background subagent), from first sight to its
-- report. Written by the hook ingest in the transaction of the hook that
-- showed it.
--   harness_job_id   the harness's own id for it (Claude's task id, a
--                    subagent's agent_id): the one its report names.
--   kind, description  as the harness reported.
--   start_call_id    the harness's id of the tool call that started it.
--   started_turn_id  the turn it was started in (as far as known: the turn
--                    whose call started it, a subagent's start, or the turn
--                    whose Stop listed it running).
--   report_turn_id   the turn its report opened or was delivered into.
--   start_basis      how started_turn_id is known: reported (a hook named
--                    the call or the subagent's start) or matched (oxplow's
--                    guess: the turn whose Stop listed it running).
--   status, summary  how it ended, in the harness's words; NULL while it
--                    runs.
--   lost_at          when its session ended or restarted with no report:
--                    oxplow's own fact, not the harness's status.
-- A subagent resumed under the same id after it reported is a new row.
CREATE TABLE agent_job (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    agent_session_id INTEGER REFERENCES agent_session(id) ON DELETE SET NULL,
    harness_job_id TEXT NOT NULL,
    kind TEXT,
    description TEXT,
    start_call_id TEXT,
    started_turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL,
    start_basis TEXT,
    seen_at TEXT NOT NULL,
    report_turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL,
    reported_at TEXT,
    status TEXT,
    summary TEXT,
    lost_at TEXT
);
CREATE INDEX idx_agent_job_harness ON agent_job(thread_id, harness_job_id);
CREATE INDEX idx_agent_job_running ON agent_job(agent_session_id)
    WHERE reported_at IS NULL AND lost_at IS NULL;
CREATE INDEX idx_agent_job_started_turn ON agent_job(started_turn_id)
    WHERE started_turn_id IS NOT NULL;
