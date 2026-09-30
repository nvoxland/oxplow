SELECT id, stream_id, created_at, revision, branch, tree_hash
  FROM source('snapshot')
