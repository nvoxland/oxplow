WITH refs AS (
    SELECT uri_param(pr.source_uri, 'slug') AS slug, pr.target_uri
    FROM source('page_ref') pr
    WHERE pr.source_resource = 'oxplow/wiki'
)
SELECT CAST(uri('oxplow/wiki', 'slug', w.slug) AS TEXT) AS ref,
       -- The record is the active store's pages.
       (SELECT a.provider FROM source('capability_provider') a
         WHERE a.capability = 'knowledge' AND a.active = 1) AS provider,
       w.slug,
       w.title,
       w.body_excerpt AS excerpt,
       w.body_size_bytes AS body_size,
       CAST((SELECT json_group_array(t) FROM
           (SELECT DISTINCT r.target_uri AS t
              FROM refs r WHERE r.slug = w.slug ORDER BY t)) AS TEXT) AS outbound_refs,
       CAST((SELECT count(DISTINCT k.path) FROM ref('knowledge_pin') k
         WHERE k.slug = w.slug AND k.stale = 1) AS INTEGER) AS stale_ref_count,
       w.updated_at
FROM source('wiki_page') w
