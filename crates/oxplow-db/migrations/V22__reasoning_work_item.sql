-- A claim or decision names the work item it was made on by ref, whichever
-- work list it's on (it held an oxplow task id, a foreign key to `task`).
-- SQLite can't drop a column in a foreign key, so both tables are rebuilt;
-- nothing references them, and their ids are kept.

CREATE TABLE claim_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    -- `work_item:<provider>:<id>`; NULL when made on no item.
    work_item TEXT,
    effort_id INTEGER REFERENCES "effort"(id) ON DELETE SET NULL,
    statement TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('tests_pass', 'no_behavior_change', 'handles_case', 'other')),
    -- What backs it: `run:<capture id>`, a test name, a file, … NULL = unbacked.
    evidence_ref TEXT,
    created_at TEXT NOT NULL,
    turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL
);
INSERT INTO claim_new (id, thread_id, work_item, effort_id, statement, kind, evidence_ref,
                       created_at, turn_id)
SELECT id, thread_id,
       CASE WHEN task_id IS NULL THEN NULL ELSE 'work_item:oxplow:tsk' || task_id END,
       effort_id, statement, kind, evidence_ref, created_at, turn_id
  FROM claim;
DROP TABLE claim;
ALTER TABLE claim_new RENAME TO claim;
CREATE INDEX idx_claim_effort ON claim(effort_id);

CREATE TABLE decision_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    -- `work_item:<provider>:<id>`; NULL when made on no item.
    work_item TEXT,
    effort_id INTEGER REFERENCES "effort"(id) ON DELETE SET NULL,
    question TEXT NOT NULL,
    choice TEXT NOT NULL,
    alternatives_json TEXT NOT NULL DEFAULT '[]',
    confidence TEXT NOT NULL CHECK (confidence IN ('low', 'medium', 'high')),
    why TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    provenance TEXT NOT NULL DEFAULT 'recorded'
        CHECK (provenance IN ('recorded', 'inferred', 'confirmed', 'dismissed')),
    turn_id INTEGER REFERENCES agent_turn(id) ON DELETE SET NULL
);
INSERT INTO decision_new (id, thread_id, work_item, effort_id, question, choice,
                          alternatives_json, confidence, why, created_at, provenance, turn_id)
SELECT id, thread_id,
       CASE WHEN task_id IS NULL THEN NULL ELSE 'work_item:oxplow:tsk' || task_id END,
       effort_id, question, choice, alternatives_json, confidence, why, created_at,
       provenance, turn_id
  FROM decision;
DROP TABLE decision;
ALTER TABLE decision_new RENAME TO decision;
CREATE INDEX idx_decision_effort ON decision(effort_id);
