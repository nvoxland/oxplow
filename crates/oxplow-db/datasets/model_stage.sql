-- Each store's workflow on a shared model (`.context/models.md` "Stages"),
-- in order: what a shared screen draws a store's columns from.
SELECT s.model, s.store, CAST(s.position AS INT) AS position, s.id, s.title, s.bucket, s.extension
FROM source('model_stage_decl') s
