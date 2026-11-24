ALTER TABLE collector_run RENAME TO legacy_collector_run;

CREATE TABLE collector_checkpoint (
    owner TEXT NOT NULL,
    id TEXT NOT NULL,
    cursor_json TEXT,
    last_event_id INTEGER,
    last_run_id TEXT NOT NULL REFERENCES contribution_run(run_id),
    row_counts_json TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (owner, id)
) STRICT;

CREATE INDEX collector_checkpoint_by_run ON collector_checkpoint (last_run_id);

CREATE TABLE collector_run (
    run_id TEXT PRIMARY KEY REFERENCES contribution_run(run_id) ON DELETE CASCADE,
    owner TEXT NOT NULL,
    id TEXT NOT NULL,
    row_counts_json TEXT
) STRICT;

CREATE INDEX collector_run_by_collector ON collector_run (owner, id, run_id);

-- Some collectors only kept the checkpoint (including report collectors).
-- Recover the declaring extension name when its declaration is still present.
INSERT INTO contribution_run (
    run_id, extension, contribution, kind, actor_kind, mode,
    started_at, finished_at, outcome, error
)
SELECT 'legacy-collector:' || c.owner || '/' || c.id,
       coalesce(d.extension, c.owner),
       'oxplow:oxplow/collector?collector=' || coalesce(d.namespace, c.owner) || '/' || c.id,
       'collector', 'system', 'external', c.last_run_at, c.last_run_at,
       CASE c.status
           WHEN 'error' THEN 'failed'
           WHEN 'needs_approval' THEN 'denied'
           ELSE c.status
       END,
       c.error
FROM legacy_collector_run c
LEFT JOIN contribution d ON d.id = coalesce((
    SELECT id FROM contribution
    WHERE kind = 'collector' AND local = c.id AND namespace = c.owner
    ORDER BY id
    LIMIT 1
), (
    SELECT id FROM contribution
    WHERE kind = 'collector' AND local = c.id AND extension = c.owner
    ORDER BY id
    LIMIT 1
))
WHERE NOT EXISTS (
    SELECT 1 FROM contribution_run r WHERE r.run_id = c.run_id AND r.kind = 'collector'
);

-- A linked checkpoint supplies the collector name and, for a successful
-- latest run, its counts. Older runs never stored individual row counts.
WITH history AS (
    SELECT r.run_id, r.outcome,
           substr(r.contribution, length('oxplow:oxplow/collector?collector=') + 1) AS collector,
           c.owner AS checkpoint_owner, c.id AS checkpoint_id, c.row_counts_json
    FROM contribution_run r
    LEFT JOIN legacy_collector_run c ON c.run_id = r.run_id
        OR ('legacy-collector:' || c.owner || '/' || c.id) = r.run_id
    WHERE r.kind = 'collector'
)
INSERT INTO collector_run (run_id, owner, id, row_counts_json)
SELECT h.run_id,
       coalesce(d.namespace, substr(h.collector, 1, instr(h.collector, '/') - 1), h.checkpoint_owner),
       coalesce(h.checkpoint_id, substr(h.collector, instr(h.collector, '/') + 1)),
       CASE WHEN h.outcome = 'ok' THEN h.row_counts_json END
FROM history h
LEFT JOIN contribution_run r ON r.run_id = h.run_id
LEFT JOIN contribution d ON d.kind = 'collector'
    AND d.extension = r.extension
    AND d.local = substr(h.collector, instr(h.collector, '/') + 1);

INSERT INTO collector_checkpoint (owner, id, cursor_json, last_event_id, last_run_id, row_counts_json)
SELECT r.owner, r.id, c.cursor_json, c.last_event_id, r.run_id, c.row_counts_json
FROM legacy_collector_run c
JOIN collector_run r ON r.run_id = coalesce(
    (SELECT run_id FROM collector_run WHERE run_id = c.run_id),
    'legacy-collector:' || c.owner || '/' || c.id
);

DROP TABLE legacy_collector_run;
