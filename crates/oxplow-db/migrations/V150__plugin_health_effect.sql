-- P8.D11: an extension's effect has health too, under the same policy —
-- three failures in a row disable it until a person enables it again. The
-- kind CHECK can't change in place, so `plugin_health` is rebuilt.
CREATE TABLE plugin_health_new (
    plugin TEXT NOT NULL,
    contribution TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('provider', 'collector', 'effect')),
    state TEXT NOT NULL CHECK (state IN ('ok', 'failing', 'disabled')),
    reason TEXT,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    last_ok_at TEXT,
    last_error TEXT,
    mean_ms REAL,
    next_due_at TEXT,
    updated_at TEXT NOT NULL,
    repair_item TEXT,
    repair_seq INTEGER,
    PRIMARY KEY (plugin, kind, contribution)
) STRICT;
INSERT INTO plugin_health_new
SELECT plugin, contribution, kind, state, reason, consecutive_failures, last_ok_at,
       last_error, mean_ms, next_due_at, updated_at, repair_item, repair_seq
FROM plugin_health;
DROP TABLE plugin_health;
ALTER TABLE plugin_health_new RENAME TO plugin_health;
