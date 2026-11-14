-- Every store's knowledge pages, over the shared knowledge model, in the
-- shape core's old knowledge page view had, plus the page's `body`. A page's
-- reads are `v_oxplow_knowledge_read`'s, its stale refs the pins
-- `v_oxplow_knowledge_ref` finds behind the primary stream's latest snapshot.
WITH refs AS (
    SELECT pr.source_uri AS page, pr.target_uri
    FROM ref('page_ref') pr
    WHERE pr.source_resource = 'oxplow/knowledge'
),
-- Each page's stale pins, counted once for all pages (not once per page).
stale AS (
    SELECT p.page, count(DISTINCT p.path) AS n
    FROM ref('knowledge_ref') p
    WHERE p.stale = 1
    GROUP BY p.page
)
SELECT k.uri AS uri,
       k.store AS store,
       k.id AS id,
       CAST(COALESCE(k.kind, 'note') AS TEXT) AS kind,
       k.title AS title,
       k.url AS url,
       k.author AS author,
       CAST(replace(substr(CAST(coalesce(k.body, '') AS TEXT), 1, 280), char(10), ' ') AS TEXT) AS excerpt,
       CAST(coalesce(k.body, '') AS TEXT) AS body,
       CAST(length(CAST(coalesce(k.body, '') AS BLOB)) AS INTEGER) AS body_size,
       CAST((SELECT json_group_array(t) FROM
           (SELECT DISTINCT r.target_uri AS t
              FROM refs r WHERE r.page = k.uri ORDER BY t)) AS TEXT) AS outbound_uris,
       CAST(coalesce(st.n, 0) AS INTEGER) AS stale_ref_count,
       CAST((SELECT count(*) FROM ref('knowledge_read') r WHERE r.uri = k.uri) AS INTEGER) AS read_count,
       CAST((SELECT count(*) FROM ref('knowledge_read') r WHERE r.uri = k.uri AND r.actor = 'agent') AS INTEGER) AS agent_read_count,
       CAST((SELECT max(r.at) FROM ref('knowledge_read') r WHERE r.uri = k.uri) AS TEXT) AS last_read_at,
       k.updated_at AS updated_at
FROM ref('knowledge_record') k
LEFT JOIN stale st ON st.page = k.uri
