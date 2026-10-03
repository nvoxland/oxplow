SELECT asset, computed_at, events_to, snapshot_id, elapsed_ms, mode, watermark, row_count
FROM source('asset_state')
