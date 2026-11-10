SELECT ef.effort_id, e.task,
       ef.path, ef.change_kind, ef.closest_vcs_rev, ef.source,
       ef.agent_session_id, ef.shared, ef.claim_basis
FROM source('effort_file') ef
JOIN source('effort') e ON e.id = ef.effort_id
