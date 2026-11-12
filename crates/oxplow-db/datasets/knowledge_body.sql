SELECT p.uri, k.body
FROM source('knowledge_page') k
JOIN ref('knowledge_page') p ON p.uri = k.uri
