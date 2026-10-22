-- A resource opens as itself: the pages that render it say so (`pages[].for`),
-- so a resource no longer names one page of its own.
ALTER TABLE resource DROP COLUMN page;
