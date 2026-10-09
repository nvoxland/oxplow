SELECT id, extension, kind, local, origin, version, stability, contract_version,
       runs_on_json AS runs_on, needs_json AS needs, after_json AS after,
       global_name, aliases_json AS aliases, file, line
FROM source('contribution')
