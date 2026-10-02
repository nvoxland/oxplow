-- P7.B4: what a change's analysis was computed against. `snapshot_id` is
-- the stream's snapshot when it was computed (the head side's for a turn
-- or a closed effort; NULL for a commit); `events_to` the event log's
-- highest seq as the computation began — the `change.analyze` consumer
-- recomputes a working tree or an open effort as the stream moves, and a
-- duplicate scan only stores findings for the computation it belongs to.
ALTER TABLE change ADD COLUMN snapshot_id INTEGER;
ALTER TABLE change ADD COLUMN events_to INTEGER;
