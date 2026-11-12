-- Every pinned ref of any source (a shared model's pinned markdown column,
-- `.context/models.md` "Markdown bodies") and how current it is: a file
-- is stale when the primary stream's latest snapshot of it is newer than
-- its pin (or it was captured but never pinned); a directory when any file
-- under it is.
WITH pins AS (
  SELECT pr.source_uri, pr.source_resource, pr.ref_type, pr.local_snapshot_id,
         pr.closest_vcs_rev, pr.vcs_rev_exact, uri_param(pr.target_uri, 'path') AS path
  FROM source('page_ref') pr
  WHERE pr.ref_type IN ('file_pin', 'dir_pin')
)
SELECT CAST(p.source_uri AS TEXT) AS source,
       CAST(p.source_resource AS TEXT) AS source_resource,
       CAST(p.path AS TEXT) AS path,
       CAST(CASE p.ref_type WHEN 'dir_pin' THEN 'dir' ELSE 'file' END AS TEXT) AS kind,
       p.local_snapshot_id AS pinned_snapshot_id,
       p.closest_vcs_rev AS pinned_vcs_rev,
       CAST(p.vcs_rev_exact AS INTEGER) AS pinned_vcs_rev_exact,
       CAST(l.snapshot_id AS INTEGER) AS latest_snapshot_id,
       CAST(l.snapshot_id IS NOT NULL
            AND (p.local_snapshot_id IS NULL OR l.snapshot_id > p.local_snapshot_id)
            AS INTEGER) AS stale
FROM pins p
LEFT JOIN (SELECT p2.source_uri, p2.ref_type, p2.path, MAX(fs.snapshot_id) AS snapshot_id
             FROM pins p2
             JOIN source('file_snapshot') fs
               ON (p2.ref_type = 'file_pin' AND fs.path = p2.path)
               OR (p2.ref_type = 'dir_pin' AND fs.path LIKE p2.path || '/%')
             JOIN source('streams') st ON st.id = fs.stream_id AND st.kind = 'primary'
            GROUP BY p2.source_uri, p2.ref_type, p2.path) l
  ON l.source_uri = p.source_uri AND l.ref_type = p.ref_type AND l.path = p.path
