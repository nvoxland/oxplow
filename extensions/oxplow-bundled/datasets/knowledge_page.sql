-- Every store's knowledge pages, over the shared knowledge model, in the
-- shape core's `v_knowledge_page` had, plus the page's `body`. A page's
-- reads are an agent's (`record.read`, through a tool or the file) and a
-- person's (their page visits); its stale refs are the pins `v_ref_pin`
-- finds behind the primary stream's latest snapshot.
WITH refs AS (
    SELECT pr.source_uri AS page, pr.target_uri
    FROM ref('page_ref') pr
    WHERE pr.source_resource = 'oxplow/knowledge'
),
reads AS (
    SELECT json_extract(e.payload, '$.record') AS uri, e.at, 1 AS agent
    FROM ref('event') e
    WHERE e.type = 'record.read'
    UNION ALL
    SELECT v.page_id AS uri, v.visited_at AS at, 0 AS agent
    FROM ref('page_visit') v
    WHERE v.page_kind = 'oxplow/knowledge'
),
store AS (
    SELECT a.provider AS store, a.kinds AS kinds
    FROM ref('capability_provider') a
    WHERE a.capability = 'knowledge' AND a.active = 1
)
SELECT k.uri AS uri,
       k.store AS store,
       k.id AS id,
       CAST(COALESCE(k.kind, json_extract(s.kinds, '$[0].id'), 'note') AS TEXT) AS kind,
       CAST(COALESCE(
           (SELECT json_extract(j.value, '$.bucket') FROM json_each(s.kinds) j
             WHERE json_extract(j.value, '$.id') = k.kind),
           json_extract(s.kinds, '$[0].bucket'),
           'note') AS TEXT) AS bucket,
       k.title AS title,
       k.url AS url,
       k.author AS author,
       CAST(replace(substr(CAST(coalesce(k.body, '') AS TEXT), 1, 280), char(10), ' ') AS TEXT) AS excerpt,
       CAST(coalesce(k.body, '') AS TEXT) AS body,
       CAST(length(CAST(coalesce(k.body, '') AS BLOB)) AS INTEGER) AS body_size,
       CAST((SELECT json_group_array(t) FROM
           (SELECT DISTINCT r.target_uri AS t
              FROM refs r WHERE r.page = k.uri ORDER BY t)) AS TEXT) AS outbound_uris,
       CAST((SELECT count(DISTINCT p.path) FROM ref('knowledge_pin') p
         WHERE p.page = k.uri AND p.stale = 1) AS INTEGER) AS stale_ref_count,
       CAST((SELECT count(*) FROM reads r WHERE r.uri = k.uri) AS INTEGER) AS read_count,
       CAST((SELECT count(*) FROM reads r WHERE r.uri = k.uri AND r.agent = 1) AS INTEGER) AS agent_read_count,
       CAST((SELECT max(r.at) FROM reads r WHERE r.uri = k.uri) AS TEXT) AS last_read_at,
       k.updated_at AS updated_at
FROM ref('knowledge_record') k
LEFT JOIN store s ON s.store = k.store
