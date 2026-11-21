-- Every language the loaded extensions declare, on or off.
SELECT id, title, extension, enabled, analyzable, extensions, aliases, lsp_ids,
       lsp_package, facets, monaco
FROM source('language_decl')
