WITH refs AS (
    SELECT pr.source_id AS slug, pr.target_kind, pr.target_id
    FROM source('page_ref') pr
    WHERE pr.source_kind = 'wiki'
)
SELECT CAST('wiki:' || w.slug AS TEXT) AS ref,
       -- The record is the active store's pages.
       (SELECT a.provider FROM source('capability_provider') a
         WHERE a.capability = 'knowledge' AND a.active = 1) AS provider,
       w.slug,
       w.title,
       w.body_excerpt AS excerpt,
       w.body_size_bytes AS body_size,
       CAST((SELECT json_group_array(t) FROM
           (SELECT DISTINCT r.target_kind || ':' || r.target_id AS t
              FROM refs r WHERE r.slug = w.slug ORDER BY t)) AS TEXT) AS outbound_refs,
       CAST((SELECT count(DISTINCT k.path) FROM ref('knowledge_ref') k
         WHERE k.page = 'wiki:' || w.slug AND k.stale = 1) AS INTEGER) AS stale_ref_count,
       w.updated_at
FROM source('wiki_page') w
