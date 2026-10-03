-- P8.D8: an extension's effect runs commands as itself (`actor_kind =
-- 'effect'`). SQLite can't change a CHECK in place, so `command_audit` is
-- rebuilt. A proposal's `audit_id` (ON DELETE SET NULL) would be cleared
-- by the drop, so it's kept aside and put back.
CREATE TEMP TABLE kept_proposal_audit AS
    SELECT id, audit_id FROM command_proposal WHERE audit_id IS NOT NULL;

CREATE TABLE command_audit_new (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    at           TEXT    NOT NULL,
    command      TEXT    NOT NULL,
    actor_kind   TEXT    NOT NULL
                 CHECK (actor_kind IN ('human', 'agent', 'lens', 'system', 'effect')),
    actor_id     TEXT,
    thread_id    INTEGER,
    input_json   TEXT    NOT NULL,
    outcome      TEXT    NOT NULL CHECK (outcome IN ('ok', 'denied', 'invalid', 'error')),
    error        TEXT,
    event_id     TEXT,
    inverse_json TEXT,
    undone_by    INTEGER,
    result_json  TEXT
) STRICT;
INSERT INTO command_audit_new
SELECT id, at, command, actor_kind, actor_id, thread_id, input_json, outcome, error, event_id,
       inverse_json, undone_by, result_json
  FROM command_audit;
DROP TABLE command_audit;
ALTER TABLE command_audit_new RENAME TO command_audit;

UPDATE command_proposal
   SET audit_id = (SELECT k.audit_id FROM kept_proposal_audit k WHERE k.id = command_proposal.id)
 WHERE id IN (SELECT id FROM kept_proposal_audit);
DROP TABLE kept_proposal_audit;
