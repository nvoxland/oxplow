-- Every read of an enabled store's page: an agent's (`knowledge_page_read`,
-- through a tool or a file) and a person's (their page visits).
SELECT CAST(r.uri AS TEXT) AS uri,
       CAST('agent' AS TEXT) AS actor,
       r.thread_id,
       r.turn_id,
       CAST(r.via AS TEXT) AS via,
       CAST(r.at AS TEXT) AS at
FROM source('knowledge_page_read') r
JOIN ref('knowledge_page') p ON p.uri = r.uri
UNION ALL
SELECT CAST(v.page_id AS TEXT) AS uri,
       CAST('person' AS TEXT) AS actor,
       CAST(v.thread_id AS INTEGER) AS thread_id,
       CAST(NULL AS INTEGER) AS turn_id,
       CAST('open' AS TEXT) AS via,
       CAST(v.visited_at AS TEXT) AS at
FROM source('page_visit') v
JOIN ref('knowledge_page') p ON p.uri = v.page_id
WHERE v.page_kind = 'oxplow/knowledge'
