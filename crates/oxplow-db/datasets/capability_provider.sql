SELECT capability, provider, extension, features_json AS features, active, title, source,
       available, chosen_by, capability_title, choosable, optional, description
FROM source('capability_provider')
