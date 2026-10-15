-- What a search entry was written from: the feed's own `version` when it
-- declares one (a turn's end, a row's updated_at), else its title and
-- body's hash. A restate compares it to decide what to rewrite, and a
-- versioned feed reads the bodies of the changed entries only.
ALTER TABLE search_entry RENAME COLUMN content_hash TO version;
