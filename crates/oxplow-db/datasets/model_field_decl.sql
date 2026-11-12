-- The fields every enabled extension declares on a shared model
-- (`.context/models.md` "Fields"): a store's under its store id, another
-- extension's under its namespace. Shared screens draw fields from it.
SELECT d.model, d.namespace, d.name, d.extension, d.title, d.kind,
       d.values_json AS "values", CAST(d.read_only AS INTEGER) AS read_only
FROM source('model_field_decl') d
