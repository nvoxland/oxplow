-- A custom component acting for a person is recorded in the command audit
-- (`Actor::Component`, actor kind `component`). Rebuild the STRICT table,
-- as V54 did for jobs, because SQLite cannot widen its actor_kind CHECK
-- in place.
--
-- command_proposal.audit_id references this table with ON DELETE SET NULL.
-- Keep those links while the old parent table is replaced.
CREATE TEMP TABLE command_audit_proposal_refs AS
SELECT id AS proposal_id, audit_id
  FROM command_proposal
 WHERE audit_id IS NOT NULL;

-- Preserve AUTOINCREMENT's high-water mark, including ids above MAX(id).
CREATE TEMP TABLE command_audit_sequence AS
SELECT MAX(COALESCE((SELECT seq FROM sqlite_sequence WHERE name = 'command_audit'), 0),
           COALESCE((SELECT MAX(id) FROM command_audit), 0)) AS seq;

CREATE TABLE command_audit_new (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    at           TEXT    NOT NULL,
    command      TEXT    NOT NULL,
    actor_kind   TEXT    NOT NULL
                 CHECK (actor_kind IN ('human', 'agent', 'lens', 'component', 'system', 'effect', 'job')),
    actor_id     TEXT,
    thread_id    INTEGER,
    input_json   TEXT    NOT NULL,
    outcome      TEXT    NOT NULL CHECK (outcome IN ('ok', 'denied', 'invalid', 'error')),
    error        TEXT,
    event_id     TEXT,
    inverse_json TEXT,
    undone_by    INTEGER,
    result_json  TEXT,
    scopes_json TEXT,
    session_id INTEGER,
    run_id TEXT
) STRICT;

INSERT INTO command_audit_new (
    id, at, command, actor_kind, actor_id, thread_id, input_json, outcome,
    error, event_id, inverse_json, undone_by, result_json, scopes_json,
    session_id, run_id
)
SELECT id, at, command, actor_kind, actor_id, thread_id, input_json, outcome,
       error, event_id, inverse_json, undone_by, result_json, scopes_json,
       session_id, run_id
  FROM command_audit;

-- Views are removed before migrations, but legacy rename mode also avoids
-- rewriting any dependent view SQL while the replacement table is installed.
PRAGMA legacy_alter_table = ON;
DROP TABLE command_audit;
ALTER TABLE command_audit_new RENAME TO command_audit;
PRAGMA legacy_alter_table = OFF;

-- Recreate the exact partial index used by refused-audit retention.
CREATE INDEX command_audit_refused ON command_audit (at)
    WHERE outcome IN ('invalid', 'denied');

-- Dropping the old parent set these references to NULL. Restore only links
-- captured above, after the replacement parent exists.
UPDATE command_proposal
   SET audit_id = (
       SELECT refs.audit_id
         FROM command_audit_proposal_refs AS refs
        WHERE refs.proposal_id = command_proposal.id
   )
 WHERE id IN (SELECT proposal_id FROM command_audit_proposal_refs);

DELETE FROM sqlite_sequence WHERE name = 'command_audit';
INSERT INTO sqlite_sequence (name, seq)
SELECT 'command_audit', seq FROM command_audit_sequence;

DROP TABLE command_audit_proposal_refs;
DROP TABLE command_audit_sequence;
