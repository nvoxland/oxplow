SELECT name, kind, remote, head_sha, stream_id, updated_at, is_default
FROM source('git_branch')
