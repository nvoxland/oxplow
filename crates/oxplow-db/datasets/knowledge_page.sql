-- Every enabled store's pages; a disabled store's stay recorded, hidden.
WITH refs AS (
    SELECT pr.source_uri AS page, pr.target_uri
    FROM source('page_ref') pr
    WHERE pr.source_resource = 'oxplow/knowledge'
),
store AS (
    SELECT a.provider AS store, a.kinds_json AS kinds
    FROM source('capability_provider') a
    WHERE a.capability = 'knowledge' AND a.active = 1
)
SELECT k.uri,
       k.store,
       k.id,
       CAST(COALESCE(k.kind, json_extract(s.kinds, '$[0].id')) AS TEXT) AS kind,
       CAST(COALESCE(
           (SELECT json_extract(j.value, '$.bucket') FROM json_each(s.kinds) j
             WHERE json_extract(j.value, '$.id') = k.kind),
           json_extract(s.kinds, '$[0].bucket'),
           'note') AS TEXT) AS bucket,
       k.title,
       k.url,
       k.author,
       k.excerpt,
       k.body_size_bytes AS body_size,
       CAST((SELECT json_group_array(t) FROM
           (SELECT DISTINCT r.target_uri AS t
              FROM refs r WHERE r.page = k.uri ORDER BY t)) AS TEXT) AS outbound_uris,
       CAST((SELECT count(DISTINCT p.path) FROM ref('knowledge_pin') p
         WHERE p.page = k.uri AND p.stale = 1) AS INTEGER) AS stale_ref_count,
       k.updated_at
FROM source('knowledge_page') k
JOIN store s ON s.store = k.store
