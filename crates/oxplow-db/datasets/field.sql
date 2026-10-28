SELECT c.view,
       CAST(j.key AS INTEGER) AS position,
       json_extract(j.value, '$.name') AS name,
       json_extract(j.value, '$.type') AS sql_type,
       json_extract(j.value, '$.doc') AS doc,
       (SELECT CAST(k.key AS INTEGER) + 1 FROM json_each(c.key_json) k
         WHERE k.value = json_extract(j.value, '$.name')) AS key_part,
       json_extract(m.column_ai_context_json,
                    '$."' || json_extract(j.value, '$.name') || '"') AS ai_context
FROM source('dataset_contract') c
JOIN source('dataset') m ON m.view = c.view AND m.version = c.version,
     json_each(c.columns_json) j
