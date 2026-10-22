SELECT id AS uri, extension, kind, local, origin, version, contract_version,
       runs_on_json AS runs_on, needs_json AS needs, after_json AS after, order_level,
       global_name, file, line
FROM source('contribution')
