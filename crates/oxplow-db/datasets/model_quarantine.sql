-- The rows a shared-model write skipped because they failed their checks
-- and have not been restated since (`.context/models.md` "Quarantine"):
-- one per (model, store, key), the latest message.
SELECT q.model, q.store, q.key, q.uri, q.message, q.source, q.at
FROM source('model_quarantine') q
