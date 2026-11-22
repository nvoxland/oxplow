ALTER TABLE command_audit RENAME TO command_run;
ALTER TABLE effect_run RENAME COLUMN audit_id TO command_run_id;

CREATE TABLE command_proposal_next (
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
    command_run_id INTEGER REFERENCES command_run(id) ON DELETE SET NULL,
    superseded_by INTEGER REFERENCES command_proposal_next(id) ON DELETE SET NULL
);

INSERT INTO command_proposal_next (
    id, created_at, command, input_json, actor_kind, actor_id, thread_id,
    stream_id, key, preview_json, dry_run_json, decision, decided_at,
    command_run_id, superseded_by
)
SELECT id, created_at, command, input_json, actor_kind, actor_id, thread_id,
       stream_id, key, preview_json, dry_run_json, decision, decided_at,
       audit_id, superseded_by
FROM command_proposal;

UPDATE sqlite_sequence
SET seq = max(seq, coalesce((SELECT seq FROM sqlite_sequence WHERE name = 'command_proposal'), 0))
WHERE name = 'command_proposal_next';

INSERT INTO sqlite_sequence (name, seq)
SELECT 'command_proposal_next', seq FROM sqlite_sequence
WHERE name = 'command_proposal'
  AND NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = 'command_proposal_next');

DROP TABLE command_proposal;
ALTER TABLE command_proposal_next RENAME TO command_proposal;
CREATE INDEX idx_command_proposal_pending ON command_proposal(decision, key);

-- Keep expired payloads empty and preserve the IDs, anchors and links.
UPDATE event_log
SET v = CASE type
          WHEN 'command.executed' THEN 6
          ELSE 3
        END,
    payload = CASE WHEN json_type(payload, '$.audit_id') IS NOT NULL
              THEN json_remove(json_set(payload, '$.command_run_id', json_extract(payload, '$.audit_id')), '$.audit_id')
              ELSE payload
              END
WHERE (type = 'command.executed' AND v = 5)
   OR (type = 'command.approved' AND v = 2)
   OR (type = 'ui.op_failed' AND v IN (1, 2));
