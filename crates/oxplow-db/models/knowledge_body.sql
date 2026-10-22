SELECT p.uri, w.body
FROM source('wiki_page') w
JOIN ref('knowledge_page') p ON p.slug = w.slug
