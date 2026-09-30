-- A dashboard tile is `query` (pinned SQL shown one way), `lens` or `text`
-- (P4.7). A metric tile becomes a query tile reading its metric's captures
-- through `metric_grid('capture')`, displayed as the metric card
-- (`display: metric`, the metric in `metric`, its card style still in
-- `viz`). The metric key column goes with the kind.
UPDATE dashboard_item
SET kind = 'query',
    options_json = json_set(
        CASE WHEN json_valid(options_json) THEN options_json ELSE '{}' END,
        '$.sql',
            'SELECT g.capture_id, g.bucket AS captured_at, MEASURE(''' || replace(metric_key, '''', '''''') || ''') AS value, NULL AS "group",' || char(10) ||
            '       c.branch, c.provenance, c.closest_git_version AS git_version, c.source' || char(10) ||
            'FROM metric_grid(''capture'') g LEFT JOIN v_capture c ON c.id = g.capture_id' || char(10) ||
            'WHERE MEASURE(''' || replace(metric_key, '''', '''''') || ''') IS NOT NULL' || char(10) ||
            'ORDER BY g.bucket DESC',
        '$.display', 'metric',
        '$.metric', metric_key)
WHERE kind = 'metric' AND metric_key IS NOT NULL;

-- A metric tile without a metric had nothing to show.
DELETE FROM dashboard_item WHERE kind = 'metric';

ALTER TABLE dashboard_item DROP COLUMN metric_key;
