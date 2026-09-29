SELECT sha, author, email, committed_at, subject, body,
       json_extract(parents_json, '$[0]') AS first_parent,
       json_array_length(parents_json) AS parent_count
FROM source('git_commit')
