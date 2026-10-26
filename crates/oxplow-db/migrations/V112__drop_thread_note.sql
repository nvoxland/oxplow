-- Thread notes are gone: an agent's own record of what it did is its
-- efforts, decisions and handoffs. The table, its page-ref edges and its
-- search rows go with them.
DELETE FROM page_ref
 WHERE source_resource LIKE '%thread_note' OR target_resource LIKE '%thread_note';
DELETE FROM search_fts
 WHERE rowid IN (SELECT rowid FROM search_entry WHERE kind LIKE '%thread_note');
DELETE FROM search_entry WHERE kind LIKE '%thread_note';
DROP TABLE thread_note;
