ALTER TABLE extension_job RENAME COLUMN run_id TO queued_by_run_id;

-- The persisted attempt is the source of truth for its lease and timing.
-- A pre-migration attempt's individual resource cost was not recorded.
WITH legacy_attempt AS (
    SELECT a.*, j.contribution_id, j.package_version, j.queued_by_run_id,
           substr(j.contribution_id, instr(j.contribution_id, '=') + 1) AS job_ref,
           c.context_json
    FROM extension_job_attempt a
    JOIN extension_job j ON j.id = a.job_id
    LEFT JOIN extension_job_context c ON c.job_id = j.id
)
INSERT INTO contribution_run (
    run_id, parent_run_id, extension, version, contribution, kind, actor_kind, actor_id,
    stream_id, thread_id, turn_id, session_id, snapshot_id, cause_event_id,
    mode, started_at, finished_at, outcome
)
SELECT 'legacy-job:' || a.job_id || ':' || a.attempt,
       p.run_id,
       coalesce(d.extension, nullif(substr(a.job_ref, 1, instr(a.job_ref, '/') - 1), ''), a.job_ref),
       a.package_version, a.contribution_id, 'job', 'job',
       a.job_id || ':' || a.attempt || ':' || a.fence,
       CAST(substr(json_extract(a.context_json, '$.anchors.stream_id'), 4) AS INTEGER),
       CAST(substr(json_extract(a.context_json, '$.anchors.thread_id'), 4) AS INTEGER),
       json_extract(a.context_json, '$.anchors.turn_id'),
       CAST(substr(json_extract(a.context_json, '$.anchors.agent_session_id'), 4) AS INTEGER),
       json_extract(a.context_json, '$.anchors.snapshot_id'),
       json_extract(a.context_json, '$.cause'),
       'external',
       strftime('%Y-%m-%dT%H:%M:%fZ', a.started_at_ms / 1000.0, 'unixepoch'),
       strftime('%Y-%m-%dT%H:%M:%fZ', a.closed_at_ms / 1000.0, 'unixepoch'),
       CASE a.outcome WHEN 'succeeded' THEN 'ok' ELSE a.outcome END
  FROM legacy_attempt a
  LEFT JOIN contribution_run p ON p.run_id = a.queued_by_run_id
  LEFT JOIN contribution d ON d.id = a.contribution_id AND d.kind = 'job';

CREATE TABLE extension_job_attempt_new (
    job_id TEXT NOT NULL REFERENCES extension_job(id),
    attempt INTEGER NOT NULL CHECK (attempt BETWEEN 1 AND 4294967295),
    fence INTEGER NOT NULL CHECK (fence = 2 * attempt - 1),
    started_at_ms INTEGER NOT NULL CHECK (started_at_ms >= 0),
    lease_until_ms INTEGER NOT NULL CHECK (lease_until_ms > started_at_ms),
    closed_at_ms INTEGER CHECK (closed_at_ms >= started_at_ms),
    outcome TEXT CHECK (outcome IN ('succeeded', 'failed', 'cancelled', 'interrupted')),
    result_json TEXT CHECK (result_json IS NULL OR
        (json_valid(result_json) AND length(CAST(result_json AS BLOB)) <= 1048576)),
    run_id TEXT NOT NULL UNIQUE REFERENCES contribution_run(run_id) ON DELETE CASCADE,
    PRIMARY KEY (job_id, attempt),
    UNIQUE (job_id, fence),
    UNIQUE (job_id, attempt, fence),
    CHECK ((closed_at_ms IS NULL) = (outcome IS NULL)),
    CHECK (result_json IS NULL OR
        (outcome IS NOT NULL AND outcome IN ('succeeded', 'failed', 'cancelled')))
) STRICT, WITHOUT ROWID;

INSERT INTO extension_job_attempt_new (
    job_id, attempt, fence, started_at_ms, lease_until_ms, closed_at_ms, outcome, result_json, run_id
)
SELECT job_id, attempt, fence, started_at_ms, lease_until_ms, closed_at_ms, outcome, result_json,
       'legacy-job:' || job_id || ':' || attempt
FROM extension_job_attempt;

-- Keep the foreign keys on existing outputs and observations pointing at
-- the final table name throughout the rebuild.
DROP TRIGGER extension_job_output_open_attempt;
DROP TABLE extension_job_attempt;
ALTER TABLE extension_job_attempt_new RENAME TO extension_job_attempt;

CREATE TRIGGER extension_job_attempt_immutable
BEFORE UPDATE ON extension_job_attempt
WHEN NEW.job_id IS NOT OLD.job_id OR NEW.attempt IS NOT OLD.attempt
    OR NEW.fence IS NOT OLD.fence OR NEW.started_at_ms IS NOT OLD.started_at_ms
    OR NEW.run_id IS NOT OLD.run_id
    OR NEW.lease_until_ms < OLD.lease_until_ms
    OR OLD.closed_at_ms IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'extension job closed or immutable attempt');
END;

CREATE TRIGGER extension_job_output_open_attempt
BEFORE INSERT ON extension_job_output
WHEN NOT EXISTS (
    SELECT 1 FROM extension_job_attempt
     WHERE job_id = NEW.job_id AND attempt = NEW.attempt AND fence = NEW.fence
       AND closed_at_ms IS NULL)
BEGIN
    SELECT RAISE(ABORT, 'extension job attempt is not open');
END;
