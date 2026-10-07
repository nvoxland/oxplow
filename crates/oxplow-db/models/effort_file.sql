SELECT ef.effort_id, e.work_item,
       ef.path, ef.change_kind, ef.closest_vcs_rev, ef.source
FROM source('effort_file') ef
JOIN source('effort') e ON e.id = ef.effort_id
