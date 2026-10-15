SELECT CAST('extension:' || name AS TEXT) AS ref, name, namespace, description, path, origin,
       version, enabled, sharing, purpose, source_git, source_sha,
       errors_json AS errors, warnings_json AS warnings
FROM source('extension_listing')
