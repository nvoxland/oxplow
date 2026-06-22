-- Retire the legacy effort_observation subsystem (epic tsk213, P1 / tsk215).
--
-- Coverage / test / static-analysis facts now live in the metric substrate
-- (metric_sample + metric_finding, dual-written since P1). The effort-review
-- panel reconstructs its rows from there
-- (CollectionService::effort_observations_from_metrics), so this table has no
-- remaining reader. Migration is pragmatic — one project uses oxplow and the DB
-- is backed up — so we drop rather than migrate the rows (the substrate already
-- holds the data going forward).
DROP TABLE IF EXISTS effort_observation;
