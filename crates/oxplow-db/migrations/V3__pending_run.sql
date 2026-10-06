-- A run deferred by pacing (tsk1092/tsk1093): a paced collector's
-- (`owner` its extension, `project` or `built-in`) or core's change
-- analysis (`owner` = `core`, `id` = `change/<kind>/<target>`). One row
-- per waiting job, with the latest event it'll run for, when it first
-- deferred and when an event last touched it. Gone once it runs.
CREATE TABLE pending_run (
    owner TEXT NOT NULL,
    id TEXT NOT NULL,
    event_seq INTEGER NOT NULL,
    since TEXT NOT NULL,
    touched TEXT NOT NULL,
    PRIMARY KEY (owner, id)
) STRICT;

-- How long a change's deep analysis took. (Its duplicate scan is timed
-- where it's recorded: `v_code_quality_scan`, scope `change <id>`.)
ALTER TABLE change ADD COLUMN elapsed_ms INTEGER;
