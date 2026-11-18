SELECT change_id, path, status, additions, deletions, is_test, staged, untracked
FROM source('change_file')
