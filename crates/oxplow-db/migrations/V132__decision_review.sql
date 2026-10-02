-- P7.C4: a reviewer confirms or dismisses an inferred decision
-- (`effort.confirm_decision` / `effort.dismiss_decision`), so
-- `provenance` gains `confirmed` and `dismissed`. SQLite can't widen a
-- CHECK in place: the table is rebuilt with every row and its index
-- (the views over it are recompiled after the migrations).
CREATE TABLE decision_reviewed (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id INTEGER NOT NULL REFERENCES threads(id) ON DELETE CASCADE,
    task_id INTEGER REFERENCES task(id) ON DELETE SET NULL,
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
INSERT INTO decision_reviewed (id, thread_id, task_id, effort_id, question, choice,
                               alternatives_json, confidence, why, created_at, provenance, turn_id)
SELECT id, thread_id, task_id, effort_id, question, choice,
       alternatives_json, confidence, why, created_at, provenance, turn_id
FROM decision;
DROP TABLE decision;
ALTER TABLE decision_reviewed RENAME TO decision;
CREATE INDEX idx_decision_effort ON decision(effort_id);
