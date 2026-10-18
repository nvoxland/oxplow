SELECT change_id, path, status, additions, deletions, zone, is_test, staged, untracked
FROM source('change_file')
