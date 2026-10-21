SELECT CAST(uri('oxplow/extension_deployment', 'name', d.name, 'hash', d.hash) AS TEXT) AS ref,
       (SELECT e.ref FROM ref('extension') e WHERE e.name = d.name) AS extension_ref,
       d.name, d.hash, d.deployed_at, d.from_stream AS stream_id, d.source, d.active,
       d.permissions_json AS permissions, d.build_run AS build
FROM source('extension_deployment') d
