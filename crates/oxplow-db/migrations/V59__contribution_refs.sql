-- Contributions are named by their refs (`.context/refs.md` "Using refs"):
-- `job:<extension>/<id>`, `collector:<extension>/<id>`, `command:<bus id>`
-- — never `<extension>/<kind>/<id>`. The stored ids and the event payloads
-- that carry one are rewritten in place; the catalog's own rows are
-- restated at boot, so they are dropped here. A command's ref is its bus
-- id, `<namespace>.<local>`: the bundled extensions' namespace is
-- `oxplow`, any other's its name with `-` as `_`.

DELETE FROM contribution;
ALTER TABLE contribution DROP COLUMN aliases_json;

CREATE TEMP TABLE contribution_ref_rename AS
WITH parts AS (
    SELECT DISTINCT old,
           substr(old, 1, instr(old, '/') - 1) AS ext,
           substr(substr(old, instr(old, '/') + 1), 1,
                  instr(substr(old, instr(old, '/') + 1), '/') - 1) AS kind,
           substr(substr(old, instr(old, '/') + 1),
                  instr(substr(old, instr(old, '/') + 1), '/') + 1) AS local
    FROM (SELECT contribution AS old FROM extension_run
          UNION SELECT contribution_id FROM extension_job)
    WHERE old NOT LIKE '%:%'
)
SELECT old,
       CASE
         WHEN kind = 'command' THEN 'command:'
              || CASE WHEN ext IN ('oxplow-bundled', 'oxplow-foundation') THEN 'oxplow'
                      ELSE replace(ext, '-', '_') END
              || '.' || local
         WHEN kind IN ('measure', 'metric', 'dimension', 'event_type', 'ref_kind')
              THEN kind || ':' || local
         ELSE kind || ':' || ext || '/' || local
       END AS new
FROM parts;

UPDATE extension_run
   SET contribution = (SELECT new FROM contribution_ref_rename WHERE old = contribution)
 WHERE contribution IN (SELECT old FROM contribution_ref_rename);

-- A job's identity is immutable; its trigger stands aside for the rename.
DROP TRIGGER extension_job_immutable;
UPDATE extension_job
   SET contribution_id = (SELECT new FROM contribution_ref_rename WHERE old = contribution_id)
 WHERE contribution_id IN (SELECT old FROM contribution_ref_rename);
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

DROP TABLE contribution_ref_rename;

-- contribution.job.completed: the run and the job by their refs.
UPDATE event_log
   SET payload = json_set(json_remove(payload, '$.job_id', '$.contribution'),
                          '$.job_run', 'job_run:' || json_extract(payload, '$.job_id'),
                          '$.job', 'job:' || replace(json_extract(payload, '$.contribution'), '/job/', '/'))
 WHERE type = 'contribution.job.completed' AND json_extract(payload, '$.job_id') IS NOT NULL;

-- contribution.disabled / .enabled: the contribution by its ref.
UPDATE event_log
   SET payload = json_set(json_remove(payload, '$.extension', '$.kind'),
                          '$.contribution',
                          json_extract(payload, '$.kind') || ':'
                            || substr(json_extract(payload, '$.extension'), length('extension:') + 1)
                            || '/' || json_extract(payload, '$.contribution'))
 WHERE type IN ('contribution.disabled', 'contribution.enabled')
   AND json_extract(payload, '$.kind') IS NOT NULL;

-- effect.result (every version): the effect by its ref.
UPDATE event_log
   SET payload = json_set(payload, '$.effect', 'effect:' || json_extract(payload, '$.effect'))
 WHERE type = 'effect.result' AND json_extract(payload, '$.effect') NOT LIKE '%:%';
