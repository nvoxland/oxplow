-- A page file is a page, a part or a query by its folder; `hidden` is gone
-- (a page with no category is not listed). The table is restated whole on
-- every extension generation, so nothing is backfilled.
ALTER TABLE page DROP COLUMN hidden;
ALTER TABLE page ADD COLUMN kind TEXT NOT NULL DEFAULT 'page';
