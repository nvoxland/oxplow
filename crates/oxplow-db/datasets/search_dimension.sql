-- What site search indexes for each dimension `v_dimension` holds: its
-- label, and in the body its description and key. Dimensions belong to the
-- project, not a stream.
SELECT CAST(uri('oxplow/dimension', 'dimension', d.key) AS TEXT) AS uri,
       CAST(coalesce(nullif(d.label, ''), d.key) AS TEXT) AS title,
       CAST(trim(coalesce(d.description, '') || ' ' || d.key) AS TEXT) AS body,
       CAST(NULL AS TEXT) AS stream_id
FROM ref('dimension') d
