-- tsk896: a restated kind writes only the entries that changed. Each
-- model-indexed entry keeps the hash of its title and body; the kind-wide
-- digest (V158) that let an unchanged kind skip, but rewrote the whole
-- kind on any edit, goes. A file entry (the indexer's) keeps none.
ALTER TABLE search_entry ADD COLUMN content_hash TEXT;
DROP TABLE search_kind_state;
