-- The seq of the latest `change.analyzed` event of phase `files` for the
-- change: what a review check compares its last run against, read from the
-- row instead of a scan of every analysed event's payload.
ALTER TABLE change ADD COLUMN files_analyzed_seq INTEGER;
UPDATE change SET files_analyzed_seq = (
  SELECT max(e.seq) FROM event_log e
   WHERE e.type = 'change.analyzed'
     AND json_extract(e.payload, '$.phase') = 'files'
     AND CAST(json_extract(e.payload, '$.change_id') AS INTEGER) = change.id);
-- Kept by the log itself, whoever appends the event.
CREATE TRIGGER change_files_analyzed_seq AFTER INSERT ON event_log
WHEN NEW.type = 'change.analyzed' AND json_extract(NEW.payload, '$.phase') = 'files'
BEGIN
  UPDATE change SET files_analyzed_seq = NEW.seq
   WHERE id = CAST(json_extract(NEW.payload, '$.change_id') AS INTEGER);
END;
-- A duplicate list is read by its change.
CREATE INDEX idx_change_duplicate_change ON change_duplicate (change_id);
