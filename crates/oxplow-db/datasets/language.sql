-- Every language the loaded extensions declare, on or off. A language a
-- person turned off by itself (`oxplow.contribution.disable`) is off while
-- its extension is on.
SELECT l.id, l.title, l.extension,
       CAST(l.enabled AND NOT EXISTS (
         SELECT 1 FROM source('contribution_health') h
           JOIN ref('extension') e ON e.name = l.extension
          WHERE h.kind = 'language' AND h.state = 'disabled'
            AND h.contribution = l.id AND h.namespace = e.namespace) AS INTEGER) AS enabled,
       l.analyzable, l.extensions, l.aliases, l.lsp_ids,
       l.lsp_package, l.facets, l.monaco
FROM source('language_decl') l
