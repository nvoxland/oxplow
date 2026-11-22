-- Align historical core registrations in the oxplow namespace with the
-- Foundation owner used for migrations and published command declarations.
UPDATE contribution_run
SET extension = 'oxplow-foundation'
WHERE kind = 'command' AND extension = 'core'
  AND run_id IN (SELECT run_id FROM command_run WHERE command LIKE 'oxplow.%');
