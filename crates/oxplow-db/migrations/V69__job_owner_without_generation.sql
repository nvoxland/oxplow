-- A job belongs to its extension's version and its declaration's
-- contract version, not to the catalog generation it was queued under:
-- another extension's change no longer strands it. SQLite can't drop a
-- column a CHECK names, so the table is rebuilt.
CREATE TABLE extension_job_new (
    id TEXT PRIMARY KEY NOT NULL,
    contribution_id TEXT NOT NULL CHECK (length(CAST(contribution_id AS BLOB)) BETWEEN 1 AND 512),
    package_version TEXT,
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

INSERT INTO extension_job_new (
       id, contribution_id, package_version, contract_version, operation, invoker,
       run_id, intent_key, replay_policy, state, attempt, fence, lease_until_ms,
       cancellation_requested_at_ms, created_at_ms, updated_at_ms, revision,
       request_json)
SELECT id, contribution_id, package_version, contract_version, operation, invoker,
       run_id, intent_key, replay_policy, state, attempt, fence, lease_until_ms,
       cancellation_requested_at_ms, created_at_ms, updated_at_ms, revision,
       request_json
  FROM extension_job;

-- Views are removed before migrations, but legacy rename mode also avoids
-- rewriting any dependent trigger SQL while the replacement table is installed.
PRAGMA legacy_alter_table = ON;
DROP TABLE extension_job;
ALTER TABLE extension_job_new RENAME TO extension_job;
PRAGMA legacy_alter_table = OFF;

CREATE INDEX extension_job_recovery ON extension_job (state, updated_at_ms, id);

CREATE TRIGGER extension_job_immutable
BEFORE UPDATE ON extension_job
WHEN OLD.state IN ('succeeded', 'failed', 'cancelled')
    OR NEW.id IS NOT OLD.id
    OR NEW.contribution_id IS NOT OLD.contribution_id
    OR NEW.package_version IS NOT OLD.package_version
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
