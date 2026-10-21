SELECT r.resource, r.extension, r.label, r.params, r.additional, r.resolve, r.title,
       r.searchable, r.icon, r.wikilink, r.page
FROM source('resource') r
