-- Pending missing-report messages belong in Alerts, including those queued
-- before collection diagnostics stopped entering agent context.
UPDATE agent_nudge
SET audience = 'person'
WHERE kind = 'report-less-run' AND audience = 'agent' AND delivered_at IS NULL;
