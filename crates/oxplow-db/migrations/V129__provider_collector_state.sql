-- P7.A3: where each provider instance's collector left off. `state_json`
-- is the provider's own opaque checkpoint (its last `$/state`), stored in
-- the same transaction as the records it covers, so a read that fails
-- midway resumes from the last batch that landed. `records` counts what
-- reads have delivered; `status` is the last read's outcome.
CREATE TABLE provider_collector_state (
    instance TEXT NOT NULL,
    collector TEXT NOT NULL,
    state_json TEXT,
    status TEXT NOT NULL DEFAULT 'never' CHECK (status IN ('never', 'reading', 'ok', 'error')),
    error TEXT,
    last_read_at TEXT,
    records INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (instance, collector)
) STRICT;
