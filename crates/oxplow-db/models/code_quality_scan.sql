SELECT id, tool, scope, status, started_at, ended_at, error,
       revision, file_filter
FROM source('code_quality_scan')
