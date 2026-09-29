SELECT sha, path, status, additions, deletions
FROM source('git_commit_file')
