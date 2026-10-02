-- P7.C1: each plugin contribution's health on this machine — a provider
-- instance, a collector — under one failure policy (`plugin_health.rs`).
-- `consecutive_failures` reaching the limit disables it (`state =
-- 'disabled'`, `reason`) until a person runs `plugin.enable`; the
-- transitions are logged as `plugin.disabled@1` / `plugin.enabled@1`.
-- `next_due_at` is when it should next run (a schedule's next slot), so
-- `v_plugin_health.fresh` can say a missed one is late.
CREATE TABLE plugin_health (
    plugin TEXT NOT NULL,
    contribution TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('provider', 'collector')),
    state TEXT NOT NULL CHECK (state IN ('ok', 'failing', 'disabled')),
    reason TEXT,
    consecutive_failures INTEGER NOT NULL DEFAULT 0,
    last_ok_at TEXT,
    last_error TEXT,
    mean_ms REAL,
    next_due_at TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (plugin, contribution)
) STRICT;
