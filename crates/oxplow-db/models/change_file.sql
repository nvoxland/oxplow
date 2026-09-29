SELECT change_id, path, status, additions, deletions, zone, is_test, interest, interest_reasons
FROM source('change_file')
