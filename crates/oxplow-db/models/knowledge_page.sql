WITH refs AS (
    SELECT pr.source_id AS slug,
           pr.target_kind,
           pr.target_id,
           pr.local_snapshot_id AS pinned,
           CASE WHEN pr.target_kind = 'file' THEN
               (SELECT MAX(fs.snapshot_id) FROM source('file_snapshot') fs
                 WHERE fs.path = pr.target_id)
           END AS latest
    FROM source('page_ref') pr
    WHERE pr.source_kind = 'wiki'
)
SELECT CAST('wiki:' || w.slug AS TEXT) AS ref,
       CAST('oxplow' AS TEXT) AS provider,
       w.slug,
       w.title,
       w.body_excerpt AS excerpt,
       w.body_size_bytes AS body_size,
       CAST((SELECT json_group_array(t) FROM
           (SELECT DISTINCT r.target_kind || ':' || r.target_id AS t
              FROM refs r WHERE r.slug = w.slug ORDER BY t)) AS TEXT) AS outbound_refs,
       CAST((SELECT count(DISTINCT r.target_id) FROM refs r
         WHERE r.slug = w.slug AND r.latest IS NOT NULL
           AND (r.pinned IS NULL OR r.latest > r.pinned)) AS INTEGER) AS stale_ref_count,
       w.updated_at
FROM source('wiki_page') w
