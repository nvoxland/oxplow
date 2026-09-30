SELECT CAST('wiki:' || pr.source_id AS TEXT) AS page,
       pr.target_id AS path,
       pr.local_snapshot_id AS pinned_snapshot_id,
       pr.closest_vcs_rev AS pinned_vcs_rev,
       CAST(pr.vcs_rev_exact AS INTEGER) AS pinned_vcs_rev_exact,
       CAST(latest.snapshot_id AS INTEGER) AS latest_snapshot_id,
       CAST(latest.snapshot_id IS NOT NULL
            AND (pr.local_snapshot_id IS NULL OR latest.snapshot_id > pr.local_snapshot_id)
            AS INTEGER) AS stale
FROM source('page_ref') pr
LEFT JOIN (SELECT fs.path, MAX(fs.snapshot_id) AS snapshot_id
             FROM source('file_snapshot') fs
             JOIN source('streams') st ON st.id = fs.stream_id AND st.kind = 'primary'
            GROUP BY fs.path) latest
       ON latest.path = pr.target_id
WHERE pr.source_kind = 'wiki' AND pr.target_kind = 'file'
