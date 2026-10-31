-- What site search indexes for each measure `v_measure` holds: its title,
-- and in the body its description, key and unit. Measures belong to the
-- project, not a stream.
SELECT CAST(uri('oxplow/measure', 'measure', m.key) AS TEXT) AS uri,
       CAST(coalesce(nullif(m.title, ''), m.key) AS TEXT) AS title,
       CAST(trim(coalesce(m.description, '') || ' ' || m.key || ' ' || coalesce(m.unit, '')) AS TEXT) AS body,
       CAST(NULL AS TEXT) AS stream_id
FROM ref('measure') m
