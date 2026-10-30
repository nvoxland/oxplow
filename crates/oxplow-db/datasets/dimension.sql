SELECT key, label, value_type, subject_kind, vocabulary_json, scope, promoted, entity_json, extension, ai_context_json AS ai_context, description
FROM source('dimension')
