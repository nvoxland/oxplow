SELECT id, tool, scope, status, started_at, ended_at, error,
       tree_version_kind, tree_version_value, file_filter
FROM source('code_quality_scan')
