-- `v_change_fact` finds the capture of one producer at one snapshot: the
-- delta captures that restated a path are the producer's captures at the
-- snapshots that hold the path.
CREATE INDEX idx_metric_capture_stream_producer_snapshot
  ON metric_capture(stream_id, producer, snapshot_id);
