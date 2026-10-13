SELECT id, stream_id, created_at, revision, branch, tree_hash,
       CAST('snapshot:' || id AS TEXT) AS ref
  FROM source('snapshot')
