SELECT page_uri, position, name, type, role, label, default_json AS "default", options_json AS options
FROM source('page_param')
