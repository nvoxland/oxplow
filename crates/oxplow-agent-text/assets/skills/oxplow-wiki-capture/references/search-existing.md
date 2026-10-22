# Finding an existing page

Before writing, find the topic page if there is one.

1. `oxplow_query_sql` over `v_knowledge_page`: every page's `slug`, `title`,
   `excerpt`, `outbound_uris` and `stale_ref_count`. Cheap, scan first:
   `SELECT slug, title FROM v_knowledge_page WHERE title LIKE '%hook%'`.
2. `oxplow_search` with `resources: ["oxplow/wiki"]`: full text over page bodies; catches
   pages that discuss the topic but aren't named after it.
3. `oxplow_query_sql` over `v_page_ref`: the edges between pages and records
   (`source_uri`, `target_uri`, `ref_type`). For each non-trivial file you
   read this turn, see whether a wiki page (or any other source) already
   references it: `WHERE target_uri = 'oxplow:oxplow/file?path=<path>'`, plus
   `AND source_resource = 'oxplow/wiki'` for wiki pages only. What a page points at:
   `WHERE source_uri = 'oxplow:oxplow/wiki?slug=<slug>'`.

If a clearly relevant page exists, append a new dated section to it.
Only create a page when none fits.
