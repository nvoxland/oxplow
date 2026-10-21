SELECT id, stream_id, created_at, revision, branch, tree_hash,
       CAST(uri('oxplow/snapshot', 'id', id) AS TEXT) AS ref
  FROM source('snapshot')
