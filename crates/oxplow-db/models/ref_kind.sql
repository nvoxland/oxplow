SELECT k.kind, k.extension, k.label, k.id_pattern, k.revisioned, k.wikilinks,
       k.resolve, k.page, k.icon, k.searchable
FROM source('ref_kind') k
