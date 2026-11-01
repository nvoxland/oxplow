-- What site search indexes for each dataset `v_dataset` publishes: its
-- view name, and in the body its description and owner (never the SQL).
-- Datasets belong to the project, not a stream. It reads the registry
-- (`source('dataset')`), never `v_dataset`: that joins `asset_state`, which
-- every recompute writes — this index's too — so the index would restate
-- itself without end.
SELECT CAST(uri('oxplow/dataset', 'dataset',
                CASE WHEN d.owner = 'core' THEN d.name ELSE d.owner || '/' || d.name END) AS TEXT) AS uri,
       CAST(d.view AS TEXT) AS title,
       CAST(trim(coalesce(d.description, '') || ' ' || d.name || ' ' || d.owner) AS TEXT) AS body,
       CAST(NULL AS TEXT) AS stream_id
FROM source('dataset') d
