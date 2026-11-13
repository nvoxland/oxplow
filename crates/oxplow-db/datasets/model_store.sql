-- Each live store of a shared model (`.context/models.md` "Stores"): who
-- fills it and its id pattern.
SELECT s.model, s.store, s.extension, s.id_pattern
FROM source('model_store_decl') s
