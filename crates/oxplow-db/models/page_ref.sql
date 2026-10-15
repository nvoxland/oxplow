SELECT CAST(source_kind || ':' || source_id AS TEXT) AS source_ref,
       CAST(target_kind || ':' || target_id AS TEXT) AS target_ref,
       source_kind, target_kind, ref_type, source_extra, local_snapshot_id,
       closest_vcs_rev, vcs_rev_exact
FROM source('page_ref')
