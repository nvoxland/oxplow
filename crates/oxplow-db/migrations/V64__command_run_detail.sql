-- Preserve audit IDs. An audit that borrowed a different contribution's run
-- gets its own row beneath that run. An audit with no run starts a new root.
CREATE TEMP TABLE command_run_migration AS
WITH ranked AS (
    SELECT a.id,
           a.command,
           r.run_id AS old_run_id,
           row_number() OVER (PARTITION BY r.run_id ORDER BY a.id) AS ordinal,
           r.kind,
           r.contribution,
           printf('%012x', coalesce(CAST(unixepoch(a.at, 'subsec') * 1000 AS INTEGER), 0)) AS ts,
           lower(hex(randomblob(10))) AS rand
    FROM command_audit a LEFT JOIN contribution_run r ON r.run_id = a.run_id
),
classified AS (
    SELECT *, kind = 'command'
           AND contribution = 'oxplow:oxplow/command?command=' || command
           AND ordinal = 1 AS reuse
    FROM ranked
)
SELECT id AS audit_id, old_run_id, coalesce(reuse, 0) AS reuse,
       CASE WHEN reuse THEN old_run_id
            ELSE substr(ts, 1, 8) || '-' || substr(ts, 9, 4) ||
                 '-7' || substr(rand, 1, 3) ||
                 '-' || substr('89ab', 1 + (random() & 3), 1) || substr(rand, 4, 3) ||
                 '-' || substr(rand, 7, 12)
       END AS run_id
FROM classified;

INSERT INTO contribution_run (
    run_id, parent_run_id, extension, contribution, kind,
    actor_kind, actor_id, thread_id, session_id, mode, started_at, finished_at,
    outcome, error, scopes_json
)
SELECT m.run_id, m.old_run_id,
       coalesce((SELECT known.extension FROM contribution_run known
                 WHERE known.contribution = 'oxplow:oxplow/command?command=' || a.command
                 ORDER BY known.started_at DESC LIMIT 1),
                CASE WHEN a.command LIKE 'oxplow.%' THEN 'oxplow-foundation'
                     WHEN instr(a.command, '.') > 0 THEN substr(a.command, 1, instr(a.command, '.') - 1)
                     ELSE 'core' END),
       'oxplow:oxplow/command?command=' || a.command, 'command',
       a.actor_kind, a.actor_id, a.thread_id, a.session_id, 'external', a.at, a.at,
       CASE a.outcome WHEN 'error' THEN 'failed' ELSE a.outcome END,
       a.error, coalesce(a.scopes_json, '{}')
FROM command_run_migration m JOIN command_audit a ON a.id = m.audit_id
WHERE m.reuse = 0;

UPDATE contribution_run AS r
SET outcome = CASE a.outcome WHEN 'error' THEN 'failed' ELSE a.outcome END,
    finished_at = coalesce(r.finished_at, a.at),
    error = coalesce(a.error, r.error),
    actor_kind = a.actor_kind,
    actor_id = coalesce(a.actor_id, r.actor_id),
    thread_id = coalesce(a.thread_id, r.thread_id),
    session_id = coalesce(a.session_id, r.session_id),
    scopes_json = CASE WHEN r.scopes_json = '{}' THEN coalesce(a.scopes_json, '{}')
                       ELSE r.scopes_json END
FROM command_run_migration m JOIN command_audit a ON a.id = m.audit_id
WHERE r.run_id = m.run_id AND m.reuse = 1;

CREATE TABLE command_audit_next (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    command TEXT NOT NULL,
    run_id TEXT NOT NULL UNIQUE REFERENCES contribution_run(run_id) ON DELETE CASCADE,
    input_json TEXT,
    result_json TEXT,
    event_id TEXT,
    inverse_json TEXT,
    undone_by INTEGER
) STRICT;

INSERT INTO command_audit_next
    (id, command, run_id, input_json, result_json, event_id, inverse_json, undone_by)
SELECT a.id, a.command, m.run_id, a.input_json, a.result_json, a.event_id, a.inverse_json, a.undone_by
FROM command_audit a JOIN command_run_migration m ON m.audit_id = a.id;

-- The old sequence may be higher than the last surviving audit. Preserve it
-- before generating new detail IDs so old events can't resolve to a new run.
UPDATE sqlite_sequence
SET seq = max(seq, coalesce((SELECT seq FROM sqlite_sequence WHERE name = 'command_audit'), 0))
WHERE name = 'command_audit_next';
INSERT INTO sqlite_sequence (name, seq)
SELECT 'command_audit_next', seq FROM sqlite_sequence
WHERE name = 'command_audit'
  AND NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = 'command_audit_next');

INSERT INTO command_audit_next (command, run_id)
SELECT substr(r.contribution, length('oxplow:oxplow/command?command=') + 1), r.run_id
FROM contribution_run r
WHERE r.kind = 'command'
  AND NOT EXISTS (SELECT 1 FROM command_audit_next a WHERE a.run_id = r.run_id)
ORDER BY r.started_at, r.run_id;

DROP INDEX command_audit_refused;
DROP TABLE command_audit;
ALTER TABLE command_audit_next RENAME TO command_audit;
DROP TABLE command_run_migration;

CREATE INDEX contribution_run_refused_command
ON contribution_run(started_at)
WHERE kind = 'command' AND outcome IN ('invalid', 'denied');
