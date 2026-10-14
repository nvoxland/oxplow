-- The snapshot files a capture's scan was not given (oversize, binary,
-- unreadable). A capture restates every path in its scanned set, so a path
-- it excluded is left out of that set: its last good facts stay, and this
-- row says they were not rechecked. Captures written before this recorded
-- none, so there is nothing to rewrite.
CREATE TABLE metric_capture_exclusion (
    capture_id INTEGER NOT NULL REFERENCES metric_capture(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    reason TEXT NOT NULL CHECK (reason IN ('oversize', 'binary', 'unreadable')),
    PRIMARY KEY (capture_id, path)
) WITHOUT ROWID;
