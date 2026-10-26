-- Lenses, an agent's shown answers and the database dashboards are gone:
-- a page is the one unit (its file in an extension), an agent shows a page
-- or an inline view, and a dashboard is a page whose view is a grid. Their
-- tables go, and with them what still names their URIs: bookmarks, visits,
-- remembered preferences, context items, page-ref edges and search rows.
-- Nothing is rewritten: a lens's page has a different name and params.
DELETE FROM bookmark
 WHERE ref LIKE 'oxplow:oxplow/lens?%' OR ref LIKE 'oxplow:oxplow/answer?%'
    OR ref LIKE 'oxplow:oxplow/dashboard?%';
DELETE FROM page_visit
 WHERE page_id LIKE 'oxplow:oxplow/lens?%' OR page_id LIKE 'oxplow:oxplow/answer?%'
    OR page_id LIKE 'oxplow:oxplow/dashboard?%';
DELETE FROM ui_preference
 WHERE scope LIKE 'oxplow:oxplow/lens?%' OR scope LIKE 'oxplow:oxplow/answer?%'
    OR scope LIKE 'oxplow:oxplow/dashboard?%';
DELETE FROM thread_context_item
 WHERE source_uri LIKE 'oxplow:oxplow/lens?%' OR source_uri LIKE 'oxplow:oxplow/answer?%'
    OR source_uri LIKE 'oxplow:oxplow/dashboard?%';
DELETE FROM page_ref
 WHERE source_resource LIKE '%lens' OR target_resource LIKE '%lens'
    OR source_resource LIKE '%answer' OR target_resource LIKE '%answer'
    OR source_resource LIKE '%dashboard' OR target_resource LIKE '%dashboard';
DELETE FROM search_fts
 WHERE rowid IN (SELECT rowid FROM search_entry
                  WHERE kind LIKE '%lens' OR kind LIKE '%dashboard');
DELETE FROM search_entry WHERE kind LIKE '%lens' OR kind LIKE '%dashboard';
DROP TABLE dashboard_item;
DROP TABLE dashboard;
DROP TABLE thread_answer;
DROP TABLE lens;
