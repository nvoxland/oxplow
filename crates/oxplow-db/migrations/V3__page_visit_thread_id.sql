-- The rail HUD's History section shows recently-visited pages and
-- accepts a `threadId` prop, but until now `page_visit` had no thread
-- column — so History was effectively global. Add `thread_id` so
-- list_recent / list_top can filter to one thread.
--
-- Nullable: pre-migration rows have no thread context, and there are
-- legitimate visits with no active thread (boot screens, settings).
-- Existing rows stay NULL and read as "global" history.

ALTER TABLE page_visit ADD COLUMN thread_id INTEGER;
CREATE INDEX idx_page_visit_thread_time ON page_visit(thread_id, visited_at DESC);
