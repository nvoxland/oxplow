-- How each metric moved during a request on one stream, as its stored
-- analysis computed it (`request_analysis`, refreshed as its inputs move).
SELECT d.request_id,
       CAST(uri('oxplow/request', 'id', 'req' || d.request_id) AS TEXT) AS request,
       d.stream_id, d.key, d.title, d.unit, d.direction, d.kind, d.category, d.language, d.agg,
       d.baseline, d.current, d.delta, CAST(d.changed AS INT) AS changed,
       CAST(d.attributed_files AS INT) AS attributed_files,
       CAST(d.sample_count AS INT) AS sample_count,
       d.target, d.warn_at, d.fail_at, d.crossing,
       CAST(d.latest_capture_id AS INT) AS latest_capture_id, a.refreshed_at
  FROM source('request_metric_delta') d
  JOIN source('request_analysis') a ON a.request_id = d.request_id AND a.stream_id = d.stream_id
