-- Hints (.context/extensions.md "Advisories"): a nudge is for the agent or
-- for a person (raised in Alerts, `delivered_at` when they dismiss it), and
-- each hint's evaluations are counted per thread, beside the nudges that
-- record what it raised and what was delivered.
ALTER TABLE agent_nudge ADD COLUMN audience TEXT NOT NULL DEFAULT 'agent'
    CHECK (audience IN ('agent', 'person'));

CREATE TABLE hint_stat (
    thread_id         INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    hint              TEXT    NOT NULL,
    evaluated         INTEGER NOT NULL,
    last_evaluated_at TEXT    NOT NULL,
    PRIMARY KEY (thread_id, hint)
) STRICT;
