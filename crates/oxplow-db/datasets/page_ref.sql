SELECT source_uri, target_uri, source_resource, target_resource, ref_type, source_extra,
       local_snapshot_id, closest_vcs_rev, vcs_rev_exact
FROM source('page_ref')
