SELECT capability, provider, extension, features_json AS features, active, title, source,
       available, chosen_by
FROM source('capability_provider')
