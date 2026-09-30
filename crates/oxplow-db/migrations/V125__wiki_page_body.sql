-- P6.E2: a knowledge page's body lives in its row, written with it (the
-- command and the watcher), so the UI and the search index read the row.
-- `.oxplow/` is git-ignored, so the page file isn't readable through the
-- workspace. Clearing the hash makes the boot scan restate every page
-- from its file, which fills the column.
ALTER TABLE wiki_page ADD COLUMN body TEXT NOT NULL DEFAULT '';
UPDATE wiki_page SET body_hash = '';
-- Title and body search is the site search index (`search`); the old
-- excerpt-only FTS mirror had no reader left.
DROP TABLE IF EXISTS wiki_page_fts;
