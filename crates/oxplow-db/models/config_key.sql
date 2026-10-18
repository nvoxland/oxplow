SELECT key, doc, human_only, schema_json AS schema
FROM source('config_key')
