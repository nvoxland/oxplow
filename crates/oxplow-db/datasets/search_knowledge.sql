SELECT p.uri AS uri,
       CAST(p.title AS TEXT) AS title,
       CAST(k.body AS TEXT) AS body,
       CAST(NULL AS TEXT) AS stream_id,
       CAST(k.updated_at AS TEXT) AS version
FROM source('knowledge_page') k
JOIN ref('knowledge_page') p ON p.uri = k.uri
