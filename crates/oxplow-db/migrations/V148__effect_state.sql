-- P8.D9: where each approved effect (`<extension>/<id>`) starts reading
-- the event log. A person's approval sets `start_after_seq` to the log's
-- head, so an effect never reacts to what happened before it was approved
-- — nor, once re-approved after an edit, to what happened while it waited.
CREATE TABLE effect_state (
    effect TEXT PRIMARY KEY,
    start_after_seq INTEGER NOT NULL,
    approved_at TEXT NOT NULL
) STRICT;
