SELECT CAST('wiki:' || w.slug AS TEXT) AS ref,
       CAST(w.title AS TEXT) AS title,
       CAST(w.body AS TEXT) AS body,
       CAST(NULL AS TEXT) AS stream_id
FROM source('wiki_page') w
