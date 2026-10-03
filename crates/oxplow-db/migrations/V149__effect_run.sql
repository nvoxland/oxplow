-- P8.D10: each effect's reaction to each event, at most one
-- (`UNIQUE (effect, event_id)`: a redelivered event finds it and writes
-- nothing). A run whose commands stay in oxplow's records writes its row
-- in the run's own transaction; one with a step outside it commits a
-- `started` row first — found on redelivery, it's an interrupted run,
-- recorded `failed` and never sent again.
CREATE TABLE effect_run (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    effect TEXT NOT NULL,
    event_id TEXT NOT NULL,
    event_seq INTEGER NOT NULL,
    state TEXT NOT NULL
        CHECK (state IN ('started', 'ok', 'skipped', 'proposed', 'failed')),
    reason TEXT,
    audit_id INTEGER,
    proposal_id INTEGER,
    started_at TEXT NOT NULL,
    finished_at TEXT,
    UNIQUE (effect, event_id)
) STRICT;
CREATE INDEX effect_run_by_effect ON effect_run (effect, id);
