SELECT id, stream_id, created_at, git_commit, git_branch, tree_hash
  FROM source('snapshot')
