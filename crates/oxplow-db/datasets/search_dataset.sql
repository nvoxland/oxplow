-- What site search indexes for each dataset `v_dataset` publishes: its
-- view name, and in the body its description and owner (never the SQL).
-- Datasets belong to the project, not a stream.
SELECT CAST(uri('oxplow/dataset', 'dataset',
                CASE WHEN d.owner = 'core' THEN d.name ELSE d.owner || '/' || d.name END) AS TEXT) AS uri,
       CAST(d.view AS TEXT) AS title,
       CAST(trim(coalesce(d.description, '') || ' ' || d.name || ' ' || d.owner) AS TEXT) AS body,
       CAST(NULL AS TEXT) AS stream_id
FROM ref('dataset') d
