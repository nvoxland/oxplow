-- A collector run deferred by its trigger's pacing (tsk1092): one row per
-- collector waiting, with the latest event it'll run for, when it first
-- deferred and when an event last touched it. Gone once it runs.
CREATE TABLE collector_pending (
    owner TEXT NOT NULL,
    id TEXT NOT NULL,
    event_seq INTEGER NOT NULL,
    since TEXT NOT NULL,
    touched TEXT NOT NULL,
    PRIMARY KEY (owner, id)
) STRICT;
