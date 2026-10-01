-- P6b: a command an agent ran that needs a person's confirmation, kept
-- with its preview (and what it would have done) until a person approves
-- or declines it. A newer proposal with the same `key` supersedes a
-- pending one. Approving runs the command as the person; `audit_id` is
-- that run's audit row.
CREATE TABLE command_proposal (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at TEXT NOT NULL,
    command TEXT NOT NULL,
    input_json TEXT NOT NULL,
    actor_kind TEXT NOT NULL,
    actor_id TEXT,
    thread_id INTEGER REFERENCES threads(id) ON DELETE SET NULL,
    stream_id INTEGER REFERENCES streams(id) ON DELETE SET NULL,
    key TEXT NOT NULL,
    preview_json TEXT NOT NULL,
    dry_run_json TEXT,
    decision TEXT NOT NULL
        CHECK (decision IN ('pending', 'approved', 'declined', 'superseded')),
    decided_at TEXT,
    audit_id INTEGER REFERENCES command_audit(id) ON DELETE SET NULL,
    superseded_by INTEGER REFERENCES command_proposal(id) ON DELETE SET NULL
);
CREATE INDEX idx_command_proposal_pending ON command_proposal(decision, key);
