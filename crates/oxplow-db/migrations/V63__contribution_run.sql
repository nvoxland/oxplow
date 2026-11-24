CREATE TABLE contribution_run (
    run_id TEXT PRIMARY KEY,
    parent_run_id TEXT REFERENCES contribution_run(run_id) ON DELETE SET NULL,
    extension TEXT NOT NULL,
    contribution TEXT NOT NULL,
    kind TEXT NOT NULL,
    version TEXT,
    generation TEXT,
    actor_kind TEXT NOT NULL,
    actor_id TEXT,
    stream_id INTEGER,
    thread_id INTEGER,
    turn_id INTEGER,
    session_id INTEGER,
    snapshot_id INTEGER,
    cause_event_id TEXT,
    mode TEXT NOT NULL CHECK (mode IN ('pure', 'transactional', 'external')),
    started_at TEXT NOT NULL,
    finished_at TEXT,
    outcome TEXT CHECK (outcome IN ('ok', 'skipped', 'failed', 'denied', 'cancelled', 'invalid', 'interrupted')),
    error TEXT,
    wall_ms INTEGER,
    cpu_ms INTEGER,
    peak_bytes INTEGER,
    ai_calls INTEGER,
    scopes_json TEXT NOT NULL DEFAULT '{}',
    decisions_json TEXT NOT NULL DEFAULT '[]',
    CHECK ((finished_at IS NULL) = (outcome IS NULL))
) STRICT;

-- The legacy actor column carried an event source. Keep the identifier
-- part where that source encoded one.
INSERT INTO contribution_run (
    run_id, parent_run_id, extension, contribution, kind, version, generation,
    actor_kind, actor_id, stream_id, thread_id, turn_id, session_id,
    snapshot_id, cause_event_id, mode, started_at, finished_at, outcome, error,
    wall_ms, cpu_ms, peak_bytes, ai_calls, scopes_json, decisions_json
)
SELECT r.run_id, p.run_id, r.extension, r.contribution, r.kind, r.version, r.generation,
       r.actor_kind,
       CASE
           WHEN r.actor_kind IN ('human', 'system') OR r.actor = 'agent' THEN NULL
           WHEN r.actor_kind = 'agent' AND r.actor LIKE 'agent:%' THEN substr(r.actor, 7)
           WHEN r.actor_kind = 'effect' AND r.actor LIKE 'effect:%' THEN substr(r.actor, 8)
           WHEN instr(r.actor, '=') > 0 THEN substr(r.actor, instr(r.actor, '=') + 1)
           ELSE r.actor
       END,
       r.stream_id, r.thread_id, r.turn_id, r.agent_session_id,
       r.snapshot_id, r.cause_event_id, r.mode, r.started_at, r.finished_at, r.outcome, r.error,
       r.wall_ms, r.cpu_ms, r.peak_bytes, r.ai_calls, r.scopes_json, r.decisions_json
FROM extension_run r LEFT JOIN extension_run p ON p.run_id = r.parent_run_id;

DROP TABLE extension_run;

CREATE INDEX contribution_run_by_contribution ON contribution_run (contribution, started_at);
CREATE INDEX contribution_run_by_start ON contribution_run (started_at);
CREATE INDEX contribution_run_by_parent ON contribution_run (parent_run_id);
CREATE INDEX contribution_run_by_extension ON contribution_run (extension, started_at);
