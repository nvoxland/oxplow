ALTER TABLE effect_run RENAME TO legacy_effect_run;

CREATE TABLE effect_run (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    effect TEXT NOT NULL,
    event_id TEXT NOT NULL,
    event_seq INTEGER NOT NULL,
    attempt INTEGER NOT NULL DEFAULT 1 CHECK (attempt >= 1),
    origin TEXT NOT NULL DEFAULT 'live' CHECK (origin IN ('live', 'retry', 'backfill', 'auto')),
    audit_id INTEGER,
    proposal_id INTEGER,
    retry_at TEXT,
    resend_json TEXT,
    run_id TEXT NOT NULL UNIQUE REFERENCES contribution_run(run_id) ON DELETE CASCADE,
    UNIQUE (effect, event_id, attempt)
) STRICT;

-- Legacy claims and proposals could lack a contribution run. Preserve their
-- start and finish times and the information needed to recover and retry.
INSERT INTO contribution_run (
    run_id, extension, contribution, kind, actor_kind, actor_id, cause_event_id,
    mode, started_at, finished_at, outcome, error
)
SELECT 'legacy-effect:' || e.id,
       coalesce(d.extension, substr(e.effect, 1, instr(e.effect, '/') - 1)),
       'oxplow:oxplow/effect?effect=' || e.effect, 'effect', 'effect', e.effect,
       e.event_id, 'external', e.started_at,
       CASE WHEN e.state <> 'started' THEN coalesce(e.finished_at, e.started_at) END,
       CASE e.state WHEN 'started' THEN NULL WHEN 'proposed' THEN 'skipped' ELSE e.state END,
       e.reason
FROM legacy_effect_run e
LEFT JOIN contribution d ON d.kind = 'effect'
    AND e.effect = d.namespace || '/' || d.local
WHERE NOT EXISTS (
    SELECT 1 FROM contribution_run r WHERE r.run_id = e.run_id AND r.kind = 'effect'
);

INSERT INTO effect_run (id, effect, event_id, event_seq, attempt, origin,
                        audit_id, proposal_id, retry_at, resend_json, run_id)
SELECT e.id, e.effect, e.event_id, e.event_seq, e.attempt, e.origin,
       e.audit_id, e.proposal_id, e.retry_at, e.resend_json,
       coalesce(r.run_id, 'legacy-effect:' || e.id)
FROM legacy_effect_run e
LEFT JOIN contribution_run r ON r.run_id = e.run_id AND r.kind = 'effect';

-- Some reasons used to live only in the detail row.
UPDATE contribution_run
SET error = (SELECT e.reason FROM effect_run n JOIN legacy_effect_run e ON e.id = n.id
             WHERE n.run_id = contribution_run.run_id)
WHERE error IS NULL AND run_id IN (SELECT run_id FROM effect_run);

UPDATE sqlite_sequence
SET seq = max(seq, coalesce((SELECT seq FROM sqlite_sequence WHERE name = 'legacy_effect_run'), 0))
WHERE name = 'effect_run';

INSERT INTO sqlite_sequence (name, seq)
SELECT 'effect_run', seq FROM sqlite_sequence
WHERE name = 'legacy_effect_run' AND NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = 'effect_run');

DROP TABLE legacy_effect_run;

CREATE INDEX effect_run_by_effect ON effect_run (effect, id);
CREATE INDEX effect_run_retry_due ON effect_run (retry_at) WHERE retry_at IS NOT NULL;
