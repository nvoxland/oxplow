SELECT a.asset, s.computed_at, s.events_to, s.snapshot_id, s.elapsed_ms, s.mode, s.watermark,
       s.row_count, f.failed_at, f.error
FROM (SELECT asset FROM source('asset_state') UNION SELECT asset FROM source('asset_failure')) a
LEFT JOIN source('asset_state') s ON s.asset = a.asset
LEFT JOIN source('asset_failure') f ON f.asset = a.asset
