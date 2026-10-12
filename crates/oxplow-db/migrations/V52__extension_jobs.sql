-- Private host-managed jobs. Installation changes do not erase or relabel
-- intent, attempt or late outcome history. No published models expose payloads.
CREATE TABLE extension_job (
    id TEXT PRIMARY KEY NOT NULL,
    contribution_id TEXT NOT NULL CHECK (length(CAST(contribution_id AS BLOB)) BETWEEN 1 AND 512),
    package_version TEXT,
    generation TEXT NOT NULL,
    contract_version INTEGER CHECK (contract_version > 0 AND contract_version <= 4294967295),
    operation TEXT NOT NULL CHECK (length(CAST(operation AS BLOB)) BETWEEN 1 AND 256),
    invoker TEXT NOT NULL CHECK (length(CAST(invoker AS BLOB)) BETWEEN 1 AND 256),
    run_id TEXT NOT NULL,
    intent_key TEXT NOT NULL CHECK (length(CAST(intent_key AS BLOB)) BETWEEN 1 AND 128),
    replay_policy TEXT NOT NULL CHECK (replay_policy IN ('never', 'idempotent')),
    state TEXT NOT NULL CHECK (state IN ('queued', 'running', 'interrupted', 'succeeded', 'failed', 'cancelled')),
    attempt INTEGER NOT NULL CHECK (attempt BETWEEN 0 AND 4294967295),
    fence INTEGER NOT NULL CHECK (fence >= 0),
    lease_until_ms INTEGER,
    cancellation_requested_at_ms INTEGER,
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    updated_at_ms INTEGER NOT NULL CHECK (updated_at_ms >= created_at_ms),
    revision INTEGER NOT NULL CHECK (revision > 0),
    request_json TEXT NOT NULL CHECK (
        json_valid(request_json) AND length(CAST(request_json AS BLOB)) <= 1048576
    ),
    UNIQUE (contribution_id, intent_key),
    CHECK (package_version IS NULL OR length(CAST(package_version AS BLOB)) BETWEEN 1 AND 256),
    CHECK (length(CAST(generation AS BLOB)) BETWEEN 1 AND 256),
    CHECK (cancellation_requested_at_ms IS NULL OR
        cancellation_requested_at_ms BETWEEN created_at_ms AND updated_at_ms),
    CHECK (replay_policy != 'never' OR attempt <= 1),
    CHECK (
        (state = 'queued' AND lease_until_ms IS NULL
            AND cancellation_requested_at_ms IS NULL AND fence = 2 * attempt
            AND (attempt = 0 OR replay_policy = 'idempotent'))
        OR (state = 'running' AND attempt > 0 AND fence = 2 * attempt - 1
            AND lease_until_ms IS NOT NULL AND lease_until_ms > created_at_ms)
        OR (state = 'interrupted' AND attempt > 0 AND fence = 2 * attempt
            AND lease_until_ms IS NULL)
        OR (state IN ('succeeded', 'failed') AND attempt > 0
            AND fence = 2 * attempt - 1 AND lease_until_ms IS NULL)
        OR (state = 'cancelled' AND cancellation_requested_at_ms IS NOT NULL
            AND lease_until_ms IS NULL AND
            ((attempt = 0 AND fence = 0) OR (attempt > 0 AND fence = 2 * attempt - 1)))
    )
) STRICT, WITHOUT ROWID;

CREATE INDEX extension_job_recovery ON extension_job (state, updated_at_ms, id);

CREATE TRIGGER extension_job_immutable
BEFORE UPDATE ON extension_job
WHEN OLD.state IN ('succeeded', 'failed', 'cancelled')
    OR NEW.id IS NOT OLD.id
    OR NEW.contribution_id IS NOT OLD.contribution_id
    OR NEW.package_version IS NOT OLD.package_version
    OR NEW.generation IS NOT OLD.generation
    OR NEW.contract_version IS NOT OLD.contract_version
    OR NEW.operation IS NOT OLD.operation
    OR NEW.invoker IS NOT OLD.invoker
    OR NEW.run_id IS NOT OLD.run_id
    OR NEW.intent_key IS NOT OLD.intent_key
    OR NEW.replay_policy IS NOT OLD.replay_policy
    OR NEW.created_at_ms IS NOT OLD.created_at_ms
    OR NEW.request_json IS NOT OLD.request_json
    OR NEW.updated_at_ms < OLD.updated_at_ms
    OR NEW.revision != OLD.revision + 1
    OR (OLD.cancellation_requested_at_ms IS NOT NULL
        AND NEW.cancellation_requested_at_ms IS NOT OLD.cancellation_requested_at_ms)
BEGIN
    SELECT RAISE(ABORT, 'extension job immutable identity or revision');
END;

-- Restart provenance is independent of retained run telemetry. The host
-- validates the strict versioned context; this row never conveys authority.
CREATE TABLE extension_job_context (
    job_id TEXT PRIMARY KEY NOT NULL REFERENCES extension_job(id),
    originating_run_id TEXT NOT NULL CHECK (length(originating_run_id) = 36),
    context_json TEXT NOT NULL CHECK (COALESCE(
        json_valid(context_json)
        AND length(CAST(context_json AS BLOB)) <= 65536
        AND json_type(context_json) = 'object'
        AND json_type(context_json, '$.schema_version') = 'integer'
        AND json_extract(context_json, '$.schema_version') = 1
        AND json_type(context_json, '$.originating_run_id') = 'text'
        AND json_extract(context_json, '$.originating_run_id') = originating_run_id,
        0
    ))
) STRICT, WITHOUT ROWID;

CREATE TRIGGER extension_job_context_origin
BEFORE INSERT ON extension_job_context
WHEN NEW.originating_run_id IS NOT (SELECT run_id FROM extension_job WHERE id = NEW.job_id)
BEGIN
    SELECT RAISE(ABORT, 'extension job context origin');
END;

CREATE TRIGGER extension_job_context_immutable
BEFORE UPDATE ON extension_job_context
BEGIN
    SELECT RAISE(ABORT, 'extension job context is immutable');
END;

CREATE TABLE extension_job_attempt (
    job_id TEXT NOT NULL REFERENCES extension_job(id),
    attempt INTEGER NOT NULL CHECK (attempt BETWEEN 1 AND 4294967295),
    fence INTEGER NOT NULL CHECK (fence = 2 * attempt - 1),
    started_at_ms INTEGER NOT NULL CHECK (started_at_ms >= 0),
    lease_until_ms INTEGER NOT NULL CHECK (lease_until_ms > started_at_ms),
    closed_at_ms INTEGER CHECK (closed_at_ms >= started_at_ms),
    outcome TEXT CHECK (outcome IN ('succeeded', 'failed', 'cancelled', 'interrupted')),
    result_json TEXT CHECK (result_json IS NULL OR
        (json_valid(result_json) AND length(CAST(result_json AS BLOB)) <= 1048576)),
    PRIMARY KEY (job_id, attempt),
    UNIQUE (job_id, fence),
    UNIQUE (job_id, attempt, fence),
    CHECK ((closed_at_ms IS NULL) = (outcome IS NULL)),
    CHECK (result_json IS NULL OR
        (outcome IS NOT NULL AND outcome IN ('succeeded', 'failed', 'cancelled')))
) STRICT, WITHOUT ROWID;

CREATE TRIGGER extension_job_attempt_immutable
BEFORE UPDATE ON extension_job_attempt
WHEN NEW.job_id IS NOT OLD.job_id OR NEW.attempt IS NOT OLD.attempt
    OR NEW.fence IS NOT OLD.fence OR NEW.started_at_ms IS NOT OLD.started_at_ms
    OR NEW.lease_until_ms < OLD.lease_until_ms
    OR OLD.closed_at_ms IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'extension job closed or immutable attempt');
END;

CREATE TABLE extension_job_observation (
    id TEXT PRIMARY KEY NOT NULL,
    job_id TEXT NOT NULL,
    attempt INTEGER NOT NULL,
    fence INTEGER NOT NULL,
    observed_at_ms INTEGER NOT NULL CHECK (observed_at_ms >= 0),
    outcome TEXT NOT NULL CHECK (outcome IN ('succeeded', 'failed')),
    evidence_json TEXT CHECK (evidence_json IS NULL OR
        (json_valid(evidence_json) AND length(CAST(evidence_json AS BLOB)) <= 1048576)),
    FOREIGN KEY (job_id, attempt, fence)
        REFERENCES extension_job_attempt(job_id, attempt, fence)
) STRICT, WITHOUT ROWID;

CREATE INDEX extension_job_observation_history
    ON extension_job_observation (job_id, observed_at_ms, id);

CREATE TRIGGER extension_job_observation_immutable
BEFORE UPDATE ON extension_job_observation
BEGIN
    SELECT RAISE(ABORT, 'extension job observations are append-only');
END;
