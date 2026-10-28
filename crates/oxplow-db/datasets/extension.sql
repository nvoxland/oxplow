SELECT CAST(uri('oxplow/extension', 'name', name) AS TEXT) AS uri, name, namespace, description, path, origin,
       version, enabled, purpose, deployment, hash,
       errors_json AS errors, warnings_json AS warnings,
       permissions_json AS permissions, pending_json AS pending,
       shared
FROM source('extension_listing')
