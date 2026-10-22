SELECT p.uri AS page, k.path, k.pinned_snapshot_id, k.pinned_vcs_rev,
       k.pinned_vcs_rev_exact, k.latest_snapshot_id, k.stale
FROM ref('knowledge_pin') k
JOIN ref('knowledge_page') p ON p.slug = k.slug
