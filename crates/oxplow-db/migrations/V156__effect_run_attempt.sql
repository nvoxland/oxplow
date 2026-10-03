-- P9.D4: a reaction may be attempted again — by a person's `effect.retry`
-- of one that failed — and may come from a backfill (P9.D5) rather than
-- the live consumer. `attempt` numbers a reaction's attempts from 1 (the
-- latest is its state); `origin` says what started the attempt. The
-- UNIQUE constraint can't change in place, so `effect_run` is rebuilt.
CREATE TABLE effect_run_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    effect TEXT NOT NULL,
    event_id TEXT NOT NULL,
    event_seq INTEGER NOT NULL,
    attempt INTEGER NOT NULL DEFAULT 1 CHECK (attempt >= 1),
    origin TEXT NOT NULL DEFAULT 'live' CHECK (origin IN ('live', 'retry', 'backfill')),
    state TEXT NOT NULL
        CHECK (state IN ('started', 'ok', 'skipped', 'proposed', 'failed')),
    reason TEXT,
    audit_id INTEGER,
    proposal_id INTEGER,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    UNIQUE (effect, event_id, attempt)
) STRICT;
INSERT INTO effect_run_new
    (id, effect, event_id, event_seq, state, reason, audit_id, proposal_id, started_at, finished_at)
SELECT id, effect, event_id, event_seq, state, reason, audit_id, proposal_id, started_at, finished_at
FROM effect_run;
DROP TABLE effect_run;
ALTER TABLE effect_run_new RENAME TO effect_run;
CREATE INDEX effect_run_by_effect ON effect_run (effect, id);
