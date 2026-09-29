SELECT id, dashboard_id, sort_index, kind, metric_key, options_json
FROM source('dashboard_item')
