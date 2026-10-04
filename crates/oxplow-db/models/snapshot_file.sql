SELECT id, snapshot_id, stream_id, path, storage, size_bytes, content_hash, captured_at
  FROM source('file_snapshot')
