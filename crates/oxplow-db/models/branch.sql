SELECT name, kind, remote, head_sha, stream_id, updated_at
FROM source('git_branch')
