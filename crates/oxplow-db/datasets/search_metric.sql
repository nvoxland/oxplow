-- What site search indexes for each metric `v_metric_spec` holds: its title,
-- and in the body its description, key and unit. Metrics belong to the
-- project, not a stream.
SELECT CAST(uri('oxplow/metric', 'metric', m.key) AS TEXT) AS uri,
       CAST(coalesce(nullif(m.title, ''), m.key) AS TEXT) AS title,
       CAST(trim(coalesce(m.description, '') || ' ' || m.key || ' ' || coalesce(m.unit, '')) AS TEXT) AS body,
       CAST(NULL AS TEXT) AS stream_id
FROM ref('metric_spec') m
