SELECT change_id, path, status, additions, deletions, zone, is_test
FROM source('change_file')
