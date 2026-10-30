SELECT r.resource, r.extension, r.label, r.params, r.additional, r.resolve, r.title,
       r.searchable, r.icon, r.wikilink, r.description
FROM source('resource') r
