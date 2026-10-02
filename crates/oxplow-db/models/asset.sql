SELECT asset, computed_at, events_to, snapshot_id, elapsed_ms
FROM source('asset_state')
