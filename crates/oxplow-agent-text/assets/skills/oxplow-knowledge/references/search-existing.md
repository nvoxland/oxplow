# Finding an existing page

Before writing, find the topic page if there is one, in every store.

1. `oxplow_query_sql` over `v_oxplow_knowledge_page`: every store's pages,
   `uri`, `store`, `id`, `kind`, `bucket`, `title`, `excerpt`, `body`,
   `outbound_uris`, `stale_ref_count`, `read_count`. Cheap, scan first:
   `SELECT uri, store, title FROM v_oxplow_knowledge_page WHERE title LIKE '%hook%'`.
2. `oxplow_search` with `resources: ["oxplow/knowledge"]`: full text over
   page bodies; catches pages that discuss the topic but aren't named for it.
3. `oxplow_query_sql` over `v_page_ref`: the edges between pages and records
   (`source_uri`, `target_uri`, `ref_type`). The pages that cite a file you
   read: `WHERE target_uri = 'oxplow:oxplow/file?path=<path>' AND
   source_resource = 'oxplow/knowledge'`. What a page points at:
   `WHERE source_uri = '<the page's uri>'`.
4. `oxplow_read { uri }` a candidate: its row and its body.

If a clearly relevant page exists, add a dated section to it. Only create
a page when none fits.
