SELECT c.view,
       CAST(j.key AS INTEGER) AS position,
       json_extract(j.value, '$.name') AS name,
       json_extract(j.value, '$.type') AS sql_type,
       json_extract(j.value, '$.doc') AS doc
FROM source('model_contract') c
JOIN source('model') m ON m.view = c.view AND m.version = c.version,
     json_each(c.columns_json) j
