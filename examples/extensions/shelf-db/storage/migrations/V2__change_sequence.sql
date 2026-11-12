-- A change sequence: every write command gives the row it touches the next
-- number, and a removal keeps the row as a tombstone (deleted = 1), so a
-- collector that kept the last number it saw can state exactly what
-- changed since: the rows after it, and the removals.
ALTER TABLE book ADD COLUMN seq INTEGER NOT NULL DEFAULT 0;
ALTER TABLE book ADD COLUMN deleted INTEGER NOT NULL DEFAULT 0;
UPDATE book SET seq = rowid;
