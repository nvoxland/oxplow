-- A page's view (a filter, a pick, a line) rides in its URI's `#`, never
-- its `?` (`.context/pages-and-tabs.md` "Tab id format"). The visits
-- recorded before carried it in the `?`, so they name pages no window
-- opens any more: they go, and Recents fills again as pages are visited.
DELETE FROM page_visit;
