-- What took a snapshot and whether it kept file contents. A snapshot taken
-- by an implementation that keeps none ("Track changes only") has rows with
-- identities and no blobs; `contents = 0` tells a read of such a snapshot
-- ("never kept") apart from one retention pruned ("expired"). Older ops
-- have no provider recorded (NULL) and kept contents (the default).
ALTER TABLE snapshot_op ADD COLUMN provider TEXT;
ALTER TABLE snapshot_op ADD COLUMN contents INTEGER NOT NULL DEFAULT 1;
