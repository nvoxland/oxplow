SELECT CAST('wiki:' || w.slug AS TEXT) AS ref, w.body
FROM source('wiki_page') w
