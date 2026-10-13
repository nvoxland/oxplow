SELECT p.ref AS ref,
       CAST(p.title AS TEXT) AS title,
       CAST(w.body AS TEXT) AS body,
       CAST(NULL AS TEXT) AS stream_id
FROM source('wiki_page') w
JOIN ref('knowledge_page') p ON p.slug = w.slug
