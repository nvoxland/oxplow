-- The semantic layer's "model" is a "dataset" (V120 renamed the views and the
-- manifest keys; this renames the storage and the names stored in it).

-- The tables the published datasets are recorded in.
ALTER TABLE model RENAME TO dataset;
ALTER TABLE model_input RENAME TO dataset_input;
ALTER TABLE model_contract RENAME TO dataset_contract;
ALTER TABLE model_test RENAME TO dataset_test;

-- The contribution kind `model` is `dataset`, and its resource
-- `oxplow/model` is `oxplow/dataset` (key param `dataset`): every stored URI
-- of `oxplow:oxplow/model?model=<extension>/<name>` follows. The tables
-- restated from the extensions at every catalog load are emptied, as V99 did.
DELETE FROM contribution;
DELETE FROM resource WHERE resource = 'oxplow/model';
UPDATE contribution_health
   SET kind = 'dataset',
       contribution = replace(contribution, 'oxplow:oxplow/model?model=', 'oxplow:oxplow/dataset?dataset=')
 WHERE kind = 'model';
UPDATE extension_run
   SET kind = 'dataset',
       contribution = replace(contribution, 'oxplow:oxplow/model?model=', 'oxplow:oxplow/dataset?dataset=')
 WHERE kind = 'model' OR contribution LIKE 'oxplow:oxplow/model?model=%';

UPDATE page_ref
   SET source_uri = replace(source_uri, 'oxplow:oxplow/model?model=', 'oxplow:oxplow/dataset?dataset='),
       target_uri = replace(target_uri, 'oxplow:oxplow/model?model=', 'oxplow:oxplow/dataset?dataset='),
       source_resource = CASE source_resource WHEN 'oxplow/model' THEN 'oxplow/dataset' ELSE source_resource END,
       target_resource = CASE target_resource WHEN 'oxplow/model' THEN 'oxplow/dataset' ELSE target_resource END
 WHERE source_uri LIKE 'oxplow:oxplow/model?%' OR target_uri LIKE 'oxplow:oxplow/model?%'
    OR source_resource = 'oxplow/model' OR target_resource = 'oxplow/model';
UPDATE comment
   SET target_uri = replace(target_uri, 'oxplow:oxplow/model?model=', 'oxplow:oxplow/dataset?dataset='),
       target_resource = CASE target_resource WHEN 'oxplow/model' THEN 'oxplow/dataset' ELSE target_resource END
 WHERE target_uri LIKE 'oxplow:oxplow/model?%' OR target_resource = 'oxplow/model';
UPDATE fact_subject
   SET uri = replace(uri, 'oxplow:oxplow/model?model=', 'oxplow:oxplow/dataset?dataset='),
       kind = CASE kind WHEN 'oxplow/model' THEN 'oxplow/dataset' ELSE kind END
 WHERE uri LIKE 'oxplow:oxplow/model?%' OR kind = 'oxplow/model';
UPDATE bookmark SET uri = replace(uri, 'oxplow:oxplow/model?model=', 'oxplow:oxplow/dataset?dataset=')
 WHERE uri LIKE 'oxplow:oxplow/model?%';
UPDATE page_visit SET page_id = replace(page_id, 'oxplow:oxplow/model?model=', 'oxplow:oxplow/dataset?dataset=')
 WHERE page_id LIKE 'oxplow:oxplow/model?%';
UPDATE ui_preference SET scope = replace(scope, 'oxplow:oxplow/model?model=', 'oxplow:oxplow/dataset?dataset=')
 WHERE scope LIKE '%oxplow:oxplow/model?%';
UPDATE work_item_link
   SET from_uri = replace(from_uri, 'oxplow:oxplow/model?model=', 'oxplow:oxplow/dataset?dataset='),
       to_uri = replace(to_uri, 'oxplow:oxplow/model?model=', 'oxplow:oxplow/dataset?dataset='),
       to_kind = CASE to_kind WHEN 'oxplow/model' THEN 'oxplow/dataset' ELSE to_kind END
 WHERE from_uri LIKE 'oxplow:oxplow/model?%' OR to_uri LIKE 'oxplow:oxplow/model?%' OR to_kind = 'oxplow/model';
UPDATE task_link SET to_uri = replace(to_uri, 'oxplow:oxplow/model?model=', 'oxplow:oxplow/dataset?dataset=')
 WHERE to_uri LIKE 'oxplow:oxplow/model?%';
UPDATE claim SET evidence = replace(evidence, 'oxplow:oxplow/model?model=', 'oxplow:oxplow/dataset?dataset=')
 WHERE evidence LIKE 'oxplow:oxplow/model?%';
UPDATE thread_context_item
   SET source_uri = replace(source_uri, 'oxplow:oxplow/model?model=', 'oxplow:oxplow/dataset?dataset=')
 WHERE source_uri LIKE 'oxplow:oxplow/model?%';
UPDATE event_log SET subject = replace(subject, 'oxplow:oxplow/model?model=', 'oxplow:oxplow/dataset?dataset=')
 WHERE subject LIKE 'oxplow:oxplow/model?%';

-- The search index of the kind: an entry is its resource and URI.
UPDATE search_entry
   SET uri = replace(uri, 'oxplow:oxplow/model?model=', 'oxplow:oxplow/dataset?dataset='),
       kind = 'oxplow/dataset'
 WHERE kind = 'oxplow/model';
UPDATE asset_state SET asset = 'search:oxplow/dataset' WHERE asset = 'search:oxplow/model';
DELETE FROM asset_failure WHERE asset = 'search:oxplow/model';

-- What the stored run decisions name: a read of a published dataset was the
-- need `model`.
UPDATE extension_run
   SET decisions_json = replace(decisions_json, '"need":"model"', '"need":"dataset"')
 WHERE decisions_json LIKE '%"need":"model"%';
