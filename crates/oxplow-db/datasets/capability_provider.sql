SELECT capability, provider, extension, features_json AS features, active, title, source,
       available, chosen_by, capability_title, choosable, optional, fields_json AS fields,
       stages_json AS stages, kinds_json AS kinds, id_pattern, description
FROM source('capability_provider')
