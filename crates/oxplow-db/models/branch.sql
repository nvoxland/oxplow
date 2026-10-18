SELECT name, kind, remote, head_sha, stream_id, updated_at, is_default,
       upstream, ahead, behind
FROM source('git_branch')
