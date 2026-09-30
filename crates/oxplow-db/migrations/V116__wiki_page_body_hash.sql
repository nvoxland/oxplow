-- P5.C3: the hash of the body a wiki page row was written from. The file
-- watcher compares it to the file on disk, so a page `knowledge.write_page`
-- just wrote (row, then file) isn't synced again as a hand edit.
ALTER TABLE wiki_page ADD COLUMN body_hash TEXT NOT NULL DEFAULT '';
