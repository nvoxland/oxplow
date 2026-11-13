-- A capture may take the place of its producer's earlier captures:
-- `replace_key` names what its facts are the figures of (a request's URI),
-- and recording a capture under a key deletes the producer's earlier
-- captures under it, with their facts. A request's outcome is one capture
-- per request, placed at the request's start, so each period's point is
-- the mean of the requests that started in it.
ALTER TABLE metric_capture ADD COLUMN replace_key TEXT;
CREATE INDEX idx_metric_capture_replace ON metric_capture(producer, replace_key)
    WHERE replace_key IS NOT NULL;

-- The request outcome's history was one capture per turn, each restating
-- its request: as the per-request means the measures are now, every
-- request would count once per turn. It starts again, recorded as its
-- requests' turns end. Migrations run with foreign keys off, so nothing
-- cascades: the children go first.
CREATE TEMP TABLE v27_capture (id INTEGER PRIMARY KEY);
INSERT INTO v27_capture SELECT id FROM metric_capture WHERE producer = 'oxplow.request_outcome';
CREATE TEMP TABLE v27_measure (id INTEGER PRIMARY KEY);
INSERT INTO v27_measure SELECT id FROM measure
 WHERE key IN ('oxplow.request_tokens', 'oxplow.request_cycle_time',
               'oxplow.request_test_outcome', 'oxplow.request_time_to_green');
DELETE FROM metric_live_fact WHERE measure_id IN (SELECT id FROM v27_measure);
DELETE FROM metric_cube WHERE capture_id IN (SELECT id FROM v27_capture)
                           OR measure_id IN (SELECT id FROM v27_measure);
DELETE FROM metric_cube_state WHERE measure_id IN (SELECT id FROM v27_measure);
DELETE FROM fact_chain WHERE capture_id IN (SELECT id FROM v27_capture);
DELETE FROM fact_path_hold WHERE capture_id IN (SELECT id FROM v27_capture);
DELETE FROM fact_line WHERE fact_id IN
  (SELECT id FROM fact WHERE capture_id IN (SELECT id FROM v27_capture));
DELETE FROM fact WHERE capture_id IN (SELECT id FROM v27_capture);
DELETE FROM metric_capture_exclusion WHERE capture_id IN (SELECT id FROM v27_capture);
DELETE FROM metric_capture WHERE id IN (SELECT id FROM v27_capture);
DROP TABLE v27_capture;
DROP TABLE v27_measure;
