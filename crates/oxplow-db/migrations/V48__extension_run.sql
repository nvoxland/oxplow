-- One row per finished run of extension code (`oxplow_domain::run`,
-- `.context/data-model.md` "extension_run"): who ran it, as what
-- contribution at what version and catalog generation, what caused it and
-- where, how it ended, what it cost and what it used. Never an input, a
-- result, a credential's value or a prompt. Swept on the collector window.
CREATE TABLE extension_run (
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
        CHECK (outcome IN ('ok', 'skipped', 'failed', 'denied', 'cancelled', 'stale')),
    error TEXT,
    wall_ms INTEGER NOT NULL,
    cpu_ms INTEGER NOT NULL,
    peak_bytes INTEGER NOT NULL,
    ai_calls INTEGER NOT NULL,
    scopes_json TEXT NOT NULL,
    decisions_json TEXT NOT NULL
) STRICT;
CREATE INDEX extension_run_by_contribution ON extension_run (contribution, started_at);
CREATE INDEX extension_run_by_start ON extension_run (started_at);

-- The run each existing record belongs to.
ALTER TABLE command_audit ADD COLUMN run_id TEXT;
ALTER TABLE effect_run ADD COLUMN run_id TEXT;
ALTER TABLE collector_run ADD COLUMN run_id TEXT;
ALTER TABLE ai_call ADD COLUMN run_id TEXT;
