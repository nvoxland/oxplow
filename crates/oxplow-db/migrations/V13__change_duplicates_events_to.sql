-- Which analysis a change's duplicates were stored for: a change analyzed
-- since its last stored scan (the scan was cut off by a stop) awaits one,
-- and boot redoes it (change_analysis.rs `resume_duplicates`).
ALTER TABLE change ADD COLUMN duplicates_events_to INTEGER;

-- Analyses before this column had their scans run (the rare one a stop cut
-- off isn't knowable now): count them as scanned rather than rescan every
-- change, one whole-tree parse each, on the first boot.
UPDATE change SET duplicates_events_to = events_to WHERE status = 'done';
