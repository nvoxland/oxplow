SELECT change_id, path, container, name, status, signature_changed, body_changed, start_line,
       visibility, is_test, complexity, length, params_before, params_after, complexity_delta,
       length_delta, added_lines, deleted_lines, modified_lines, churn_share
FROM source('change_function')
