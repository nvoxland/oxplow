-- P9.D2: a collector's run for an event may be skipped by the loop guard
-- (the event came from its own run, or from a chain of reactions already
-- at the limit). That is neither `ok` nor `error`. The status CHECK can't
-- change in place, so `collector_run` is rebuilt.
CREATE TABLE collector_run_new (
    owner TEXT NOT NULL,
    id TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('ok', 'error', 'needs_approval', 'skipped')),
    last_run_at TEXT NOT NULL,
    error TEXT,
    -- {"<entity>": <row count>, …} from the last successful run.
    row_counts_json TEXT NOT NULL DEFAULT '{}',
    cursor_json TEXT,
    last_event_id INTEGER,
    PRIMARY KEY (owner, id)
) STRICT;
INSERT INTO collector_run_new
SELECT owner, id, status, last_run_at, error, row_counts_json, cursor_json, last_event_id
FROM collector_run;
DROP TABLE collector_run;
ALTER TABLE collector_run_new RENAME TO collector_run;
