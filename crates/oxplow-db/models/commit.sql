SELECT sha, author, email, committed_at, subject, body,
       json_extract(parents_json, '$[0]') AS first_parent,
       json_array_length(parents_json) AS parent_count,
       parents_json AS parents,
       CAST(uri('oxplow/commit', 'sha', sha) AS TEXT) AS ref
FROM source('git_commit')
