SELECT f.id, f.snapshot_id, f.stream_id, f.path, f.storage, f.size_bytes, f.content_hash, f.captured_at,
       CAST(CASE
         WHEN f.storage = 'deleted' THEN
           CASE WHEN p.storage IS NULL OR p.storage = 'deleted' THEN NULL ELSE 'deleted' END
         WHEN p.storage IS NULL OR p.storage = 'deleted' THEN 'added'
         WHEN COALESCE(f.content_hash,
                CASE f.storage
                  WHEN 'oxplow' THEN f.blob_hash
                  WHEN 'git' THEN 'git:' || f.blob_hash
                  WHEN 'oversize' THEN 'oversize:' || f.size_bytes || ':' || COALESCE(f.mtime_ms, 0)
                END)
              = COALESCE(p.content_hash,
                CASE p.storage
                  WHEN 'oxplow' THEN p.blob_hash
                  WHEN 'git' THEN 'git:' || p.blob_hash
                  WHEN 'oversize' THEN 'oversize:' || p.size_bytes || ':' || COALESCE(p.mtime_ms, 0)
                END) THEN NULL
         ELSE 'modified'
       END AS TEXT) AS kind
  FROM source('file_snapshot') f
  LEFT JOIN source('file_snapshot') p
    ON p.id = (SELECT q.id FROM source('file_snapshot') q
                WHERE q.stream_id = f.stream_id AND q.path = f.path AND q.id < f.id
                ORDER BY q.id DESC LIMIT 1)
