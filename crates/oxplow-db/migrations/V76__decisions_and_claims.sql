-- Decisions and claims (tsk295): the agent's reasoning as reviewable data.
-- A decision is a fork the agent resolved; a claim is something it asserted
-- ("tests pass"). Recorded via MCP record_decision / record_claim.

CREATE TABLE decision (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    task_id INTEGER REFERENCES task(id) ON DELETE SET NULL,
    effort_id INTEGER REFERENCES task_effort(id) ON DELETE SET NULL,
    question TEXT NOT NULL,
    choice TEXT NOT NULL,
    alternatives_json TEXT NOT NULL DEFAULT '[]',
    confidence TEXT NOT NULL CHECK (confidence IN ('low', 'medium', 'high')),
    why TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL
);
CREATE INDEX idx_decision_effort ON decision(effort_id);

CREATE TABLE claim (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    task_id INTEGER REFERENCES task(id) ON DELETE SET NULL,
    effort_id INTEGER REFERENCES task_effort(id) ON DELETE SET NULL,
    statement TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('tests_pass', 'no_behavior_change', 'handles_case', 'other')),
    -- What backs it: `run:<capture id>`, a test name, a file, … NULL = unbacked.
    evidence_ref TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_claim_effort ON claim(effort_id);

CREATE VIEW v_decision AS
SELECT id, thread_id, task_id, effort_id, question, choice,
       alternatives_json AS alternatives, confidence, why, created_at
FROM decision;

-- `verified`: 1 when the claim cites evidence, or when it's a `tests_pass`
-- claim and its effort has a test report (producer `tests`) with no failed
-- test cases. 0 otherwise — an unverified claim is what a reviewer checks.
CREATE VIEW v_claim AS
SELECT c.id, c.thread_id, c.task_id, c.effort_id, c.statement, c.kind,
       c.evidence_ref,
       CASE
         WHEN c.evidence_ref IS NOT NULL THEN 1
         WHEN c.kind = 'tests_pass' AND c.effort_id IS NOT NULL AND EXISTS (
           SELECT 1 FROM metric_capture mc
           WHERE mc.effort_id = c.effort_id AND mc.producer = 'tests' AND mc.status = 'done'
             AND EXISTS (SELECT 1 FROM fact f WHERE f.capture_id = mc.id)
             AND NOT EXISTS (
               SELECT 1 FROM fact f
               WHERE f.capture_id = mc.id
                 AND json_extract(f.dims_json, '$."oxplow.status"') = 'failed'
             )
         ) THEN 1
         ELSE 0
       END AS verified,
       c.created_at
FROM claim c;
