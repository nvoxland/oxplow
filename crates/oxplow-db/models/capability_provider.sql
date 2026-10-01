SELECT capability, provider, extension, features_json AS features, active
FROM source('capability_provider')
