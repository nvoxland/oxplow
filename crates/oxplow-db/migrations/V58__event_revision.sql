-- The code state an event is about, as a resolved revision (`git:<sha>`,
-- `snap:<id>`, `working`): what a run triggered by the event reads as its
-- tree (`change.analyzed`'s head). NULL for an event about no particular one.
ALTER TABLE event_log ADD COLUMN revision TEXT;
