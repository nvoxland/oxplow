ALTER TABLE extension_job RENAME TO job_run;
ALTER TABLE extension_job_attempt RENAME TO job_run_attempt;
ALTER TABLE extension_job_context RENAME TO job_run_context;
ALTER TABLE extension_job_observation RENAME TO job_run_observation;
ALTER TABLE extension_job_output RENAME TO job_run_output;

DROP INDEX extension_job_observation_history;
CREATE INDEX job_run_observation_history
    ON job_run_observation (job_id, observed_at_ms, id);

DROP INDEX extension_job_recovery;
CREATE INDEX job_run_recovery ON job_run (state, updated_at_ms, id);

DROP TRIGGER extension_job_context_origin;
CREATE TRIGGER job_run_context_origin
BEFORE INSERT ON job_run_context
WHEN NEW.originating_run_id IS NOT (SELECT queued_by_run_id FROM job_run WHERE id = NEW.job_id)
BEGIN
    SELECT RAISE(ABORT, 'extension job context origin');
END;

DROP TRIGGER extension_job_context_immutable;
CREATE TRIGGER job_run_context_immutable
BEFORE UPDATE ON job_run_context
BEGIN
    SELECT RAISE(ABORT, 'extension job context is immutable');
END;

DROP TRIGGER extension_job_attempt_immutable;
CREATE TRIGGER job_run_attempt_immutable
BEFORE UPDATE ON job_run_attempt
WHEN NEW.job_id IS NOT OLD.job_id OR NEW.attempt IS NOT OLD.attempt
    OR NEW.fence IS NOT OLD.fence OR NEW.started_at_ms IS NOT OLD.started_at_ms
    OR NEW.run_id IS NOT OLD.run_id
    OR NEW.lease_until_ms < OLD.lease_until_ms
    OR OLD.closed_at_ms IS NOT NULL
BEGIN
    SELECT RAISE(ABORT, 'extension job closed or immutable attempt');
END;

DROP TRIGGER extension_job_observation_immutable;
CREATE TRIGGER job_run_observation_immutable
BEFORE UPDATE ON job_run_observation
BEGIN
    SELECT RAISE(ABORT, 'extension job observations are append-only');
END;

DROP TRIGGER extension_job_output_append_only;
CREATE TRIGGER job_run_output_append_only
BEFORE UPDATE ON job_run_output
BEGIN
    SELECT RAISE(ABORT, 'extension job output is append-only');
END;

DROP TRIGGER extension_job_output_open_attempt;
CREATE TRIGGER job_run_output_open_attempt
BEFORE INSERT ON job_run_output
WHEN NOT EXISTS (
    SELECT 1 FROM job_run_attempt
     WHERE job_id = NEW.job_id AND attempt = NEW.attempt AND fence = NEW.fence
       AND closed_at_ms IS NULL)
BEGIN
    SELECT RAISE(ABORT, 'extension job attempt is not open');
END;

DROP TRIGGER extension_job_immutable;
CREATE TRIGGER job_run_immutable
BEFORE UPDATE ON job_run
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
    OR NEW.queued_by_run_id IS NOT OLD.queued_by_run_id
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
