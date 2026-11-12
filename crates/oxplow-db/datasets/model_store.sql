-- Each live store of a shared model (`.context/models.md` "Stores"): who
-- fills it, whether it is a provider instance's, and its id pattern.
SELECT s.model, s.store, s.extension, CAST(s.provider AS INT) AS provider, s.id_pattern
FROM source('model_store_decl') s
