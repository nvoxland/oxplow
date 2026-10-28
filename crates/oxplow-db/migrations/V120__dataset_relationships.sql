-- A dataset's declared relationships (`relationships: [{ from, to }]`),
-- resolved, as a JSON array — published on `v_dataset.relationships`.
-- `v_model` and `v_model_column` became `v_dataset` and `v_field`: their
-- recorded contracts go with them.
ALTER TABLE model ADD COLUMN relationships_json TEXT CHECK (relationships_json IS NULL OR json_valid(relationships_json));
DELETE FROM model_contract WHERE view IN ('v_model', 'v_model_column');
