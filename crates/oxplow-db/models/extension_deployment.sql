SELECT CAST(uri('oxplow/extension_deployment', 'name', d.name, 'hash', d.hash) AS TEXT) AS uri,
       (SELECT e.uri FROM ref('extension') e WHERE e.name = d.name) AS extension_uri,
       d.name, d.hash, d.deployed_at, d."commit" AS "commit", d.dirty, d.source, d.active,
       d.permissions_json AS permissions, d.build_run AS build
FROM source('extension_deployment') d
