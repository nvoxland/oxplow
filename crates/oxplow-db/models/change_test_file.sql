SELECT change_id, path, tests_before, tests_after, assertions_before,
       assertions_after, skips_before, skips_after
FROM source('change_test_file')
