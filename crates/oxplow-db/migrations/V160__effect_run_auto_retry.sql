-- P10: an attempt whose every step went to a provider that keeps
-- `idempotent_writes` is sent again by itself (`origin = 'auto'`); a
-- failed attempt awaiting that says when (`retry_at`, RFC 3339). The
-- CHECK can't change in place, so `effect_run` is rebuilt.
CREATE TABLE effect_run_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    effect TEXT NOT NULL,
    event_id TEXT NOT NULL,
    event_seq INTEGER NOT NULL,
    attempt INTEGER NOT NULL DEFAULT 1 CHECK (attempt >= 1),
    origin TEXT NOT NULL DEFAULT 'live'
        CHECK (origin IN ('live', 'retry', 'backfill', 'auto')),
    state TEXT NOT NULL
        CHECK (state IN ('started', 'ok', 'skipped', 'proposed', 'failed')),
    reason TEXT,
    audit_id INTEGER,
    proposal_id INTEGER,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    retry_at TEXT,
    UNIQUE (effect, event_id, attempt)
) STRICT;
INSERT INTO effect_run_new
    (id, effect, event_id, event_seq, attempt, origin, state, reason, audit_id, proposal_id,
     started_at, finished_at)
SELECT id, effect, event_id, event_seq, attempt, origin, state, reason, audit_id, proposal_id,
       started_at, finished_at
FROM effect_run;
DROP TABLE effect_run;
ALTER TABLE effect_run_new RENAME TO effect_run;
CREATE INDEX effect_run_by_effect ON effect_run (effect, id);
CREATE INDEX effect_run_retry_due ON effect_run (retry_at) WHERE retry_at IS NOT NULL;
