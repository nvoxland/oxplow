-- The changed line ranges of an analysed change: one row per contiguous
-- changed range of a file. Line numbers are 1-indexed and inclusive.
-- A pure deletion has no head range: `head_after` is the head line it
-- sits after (0 = before the first line). A pure insertion has no base
-- range.
CREATE TABLE change_hunk (
    change_id INTEGER NOT NULL REFERENCES change(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    head_start INTEGER,
    head_end INTEGER,
    base_start INTEGER,
    base_end INTEGER,
    head_after INTEGER
) STRICT;
CREATE INDEX change_hunk_change_path ON change_hunk(change_id, path);
