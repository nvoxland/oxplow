-- A language is a contribution that can be turned off, like a provider, a
-- collector or an effect: `contribution_health` rows may be of kind
-- `language`. The kind check is part of the table, so it is rebuilt.
CREATE TABLE contribution_health_new (
    namespace TEXT NOT NULL,
    contribution TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('provider', 'collector', 'effect', 'language')),
    state TEXT NOT NULL CHECK (state IN ('ok', 'failing', 'disabled')),
    reason TEXT,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    last_ok_at TEXT,
    last_error TEXT,
    mean_ms REAL,
    next_due_at TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (namespace, kind, contribution)
) STRICT;
INSERT INTO contribution_health_new SELECT * FROM contribution_health;
DROP TABLE contribution_health;
ALTER TABLE contribution_health_new RENAME TO contribution_health;
