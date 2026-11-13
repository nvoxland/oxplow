-- The files and directories each knowledge page pins, and how current each
-- pin is: `v_ref_pin` of the pages (a filter of it).
SELECT p.source AS page, p.path, p.kind, p.pinned_snapshot_id, p.pinned_vcs_rev,
       p.pinned_vcs_rev_exact, p.latest_snapshot_id, p.stale
FROM ref('ref_pin') p
WHERE p.source_resource = 'oxplow/knowledge'
