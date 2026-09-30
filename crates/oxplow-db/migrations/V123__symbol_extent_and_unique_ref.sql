-- tsk571: a symbol's row keeps where its name is (`line`, `col`) and its
-- whole extent (`start_*`..`end_*`, the body included); `end_*` was the
-- name's end. Rows restate as their files change; until then a row's
-- extent is its name.
ALTER TABLE symbol ADD COLUMN start_line INTEGER NOT NULL DEFAULT 0;
ALTER TABLE symbol ADD COLUMN start_col INTEGER NOT NULL DEFAULT 0;
UPDATE symbol SET start_line = line, start_col = col;

-- Refs are unique: a name path's later symbols in a file (an overload, a
-- setter after its getter) are numbered in position order,
-- `Widget::value~2`. Existing duplicates are numbered the same way.
UPDATE symbol
SET ref = (
    SELECT replace(s.ref, '@snap:', '~' || d.nth || '@snap:')
    FROM (SELECT id, ROW_NUMBER() OVER (PARTITION BY ref ORDER BY line, col, id) AS nth
          FROM symbol) d
    JOIN symbol s ON s.id = d.id
    WHERE d.id = symbol.id
)
WHERE id IN (
    SELECT id FROM (SELECT id, ROW_NUMBER() OVER (PARTITION BY ref ORDER BY line, col, id) AS nth
                    FROM symbol)
    WHERE nth > 1
);
CREATE UNIQUE INDEX idx_symbol_ref ON symbol(ref);
