SELECT m.view, m.name, m.owner, m.kind, m.version, m.description, m.sql, m.compiled_at,
       m.materialize, m.ai_context_json AS ai_context,
       m.relationships_json AS relationships
FROM source('dataset') m
