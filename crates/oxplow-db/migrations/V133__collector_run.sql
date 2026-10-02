-- P7.B3: each collector's last run, replacing `ext_source_state`. A
-- collector is owned by an extension, by the project (`project`) or by
-- oxplow (`built-in`); its id may be dotted (`repo.scan_clone`). Besides
-- the outcome it keeps the collector's checkpoint: `cursor_json` (the
-- opaque state a script or provider read returned) and `last_event_id`
-- (the seq of the last trigger event it ran for, so redelivery writes
-- nothing).
CREATE TABLE collector_run (
    owner TEXT NOT NULL,
    id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('ok', 'error', 'needs_approval')),
    last_run_at TEXT NOT NULL,
    error TEXT,
    -- {"<entity>": <row count>, …} from the last successful run.
    row_counts_json TEXT NOT NULL DEFAULT '{}',
    cursor_json TEXT,
    last_event_id INTEGER,
    PRIMARY KEY (owner, id)
) STRICT;

INSERT INTO collector_run (owner, id, status, last_run_at, error, row_counts_json)
SELECT extension, source_id, status, last_run_at, error, row_counts_json
FROM ext_source_state;

DROP TABLE ext_source_state;
