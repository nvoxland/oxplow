-- A job belongs to the deployment whose declaration owns it, not only to
-- its extension's version: a redeploy that keeps (or has no) version is
-- another deployment. A queued job is rebound to the new one — its
-- version and deployment change together, and only then; a running job's
-- live check stops it.
ALTER TABLE extension_job ADD COLUMN deployment_hash TEXT
    CHECK (deployment_hash IS NULL OR length(CAST(deployment_hash AS BLOB)) BETWEEN 1 AND 256);

DROP TRIGGER extension_job_immutable;

CREATE TRIGGER extension_job_immutable
BEFORE UPDATE ON extension_job
WHEN OLD.state IN ('succeeded', 'failed', 'cancelled')
    OR NEW.id IS NOT OLD.id
    OR NEW.contribution_id IS NOT OLD.contribution_id
    OR ((NEW.package_version IS NOT OLD.package_version
            OR NEW.deployment_hash IS NOT OLD.deployment_hash)
        AND NOT (OLD.state = 'queued' AND NEW.state = 'queued'
            AND NEW.attempt = OLD.attempt AND NEW.fence = OLD.fence))
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
