-- No run is ever `stale`: generation staleness is the activation read
-- lock's, and nothing recorded that outcome. SQLite can't change a CHECK
-- in place, so the table is rebuilt without it.
CREATE TABLE extension_run_new (
    run_id TEXT PRIMARY KEY,
    parent_run_id TEXT,
    extension TEXT NOT NULL,
    contribution TEXT NOT NULL,
    kind TEXT NOT NULL,
    version TEXT,
    generation TEXT,
    actor_kind TEXT NOT NULL,
    actor TEXT NOT NULL,
    stream_id INTEGER,
    thread_id INTEGER,
    effort_id INTEGER,
    turn_id INTEGER,
    agent_session_id INTEGER,
    snapshot_id INTEGER,
    cause_event_id TEXT,
    mode TEXT NOT NULL CHECK (mode IN ('pure', 'transactional', 'external')),
    started_at TEXT NOT NULL,
    finished_at TEXT NOT NULL,
    outcome TEXT NOT NULL
        CHECK (outcome IN ('ok', 'skipped', 'failed', 'denied', 'cancelled')),
    error TEXT,
    wall_ms INTEGER NOT NULL,
    cpu_ms INTEGER NOT NULL,
    peak_bytes INTEGER NOT NULL,
    ai_calls INTEGER NOT NULL,
    scopes_json TEXT NOT NULL,
    decisions_json TEXT NOT NULL
) STRICT;

INSERT INTO extension_run_new SELECT * FROM extension_run;
DROP TABLE extension_run;
ALTER TABLE extension_run_new RENAME TO extension_run;
CREATE INDEX extension_run_by_contribution ON extension_run (contribution, started_at);
CREATE INDEX extension_run_by_start ON extension_run (started_at);
