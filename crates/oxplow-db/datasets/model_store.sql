-- Each live store of a shared model (`.context/models.md` "Stores"): who
-- fills it, and how a reader links its records without knowing the model.
SELECT s.model, s.store, s.extension, s.id_pattern,
       s.resource, s.store_param, s.record_key AS key, s.loose
FROM source('model_store_decl') s
